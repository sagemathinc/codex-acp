use crate::ACP_CLIENT;
use agent_client_protocol::{AgentSideConnection, Client, Error, ReadTextFileRequest, SessionId};
use async_trait::async_trait;
use codex_core::{
    FunctionCallError, ToolHandler, ToolInvocation, ToolKind, ToolOutput, ToolPayload,
    config::Config, register_external_tool_handler,
};
use codex_protocol::ConversationId;
use codex_utils_string::take_bytes_at_char_boundary;
use serde::Deserialize;
use std::collections::VecDeque;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::runtime::Handle;

pub fn register_remote_read_file_handler() {
    register_external_tool_handler("read_file", Arc::new(RemoteReadFileHandler::default()));
}

pub fn ensure_read_file_tool_enabled(config: &mut Config) {
    if !config
        .model_family
        .experimental_supported_tools
        .iter()
        .any(|tool| tool == "read_file")
    {
        config
            .model_family
            .experimental_supported_tools
            .push("read_file".to_string());
    }
}

#[derive(Default)]
struct RemoteReadFileHandler;

#[async_trait]
impl ToolHandler for RemoteReadFileHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let payload = invocation.payload.clone();
        let session_id = session_id_from_conversation_id(&invocation.conversation_id());

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "read_file handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: ReadFileArgs = serde_json::from_str(&arguments).map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "failed to parse function arguments: {err:?}"
            ))
        })?;

        let ReadFileArgs {
            file_path,
            offset,
            limit,
            mode,
            indentation,
        } = args;

        if offset == 0 {
            return Err(FunctionCallError::RespondToModel(
                "offset must be a 1-indexed line number".to_string(),
            ));
        }

        if limit == 0 {
            return Err(FunctionCallError::RespondToModel(
                "limit must be greater than zero".to_string(),
            ));
        }

        let path = PathBuf::from(&file_path);
        if !path.is_absolute() {
            return Err(FunctionCallError::RespondToModel(
                "file_path must be an absolute path".to_string(),
            ));
        }

        let lines = match mode {
            ReadMode::Slice => read_slice(&session_id, path, offset, limit).await?,
            ReadMode::Indentation => {
                let args = indentation.unwrap_or_default();
                read_indent_block(&session_id, path, offset, limit, args).await?
            }
        };

        Ok(ToolOutput::Function {
            content: lines.join("\n"),
            content_items: None,
            success: Some(true),
        })
    }
}

#[derive(Deserialize)]
struct ReadFileArgs {
    file_path: String,
    #[serde(default = "defaults::offset")]
    offset: usize,
    #[serde(default = "defaults::limit")]
    limit: usize,
    #[serde(default)]
    mode: ReadMode,
    #[serde(default)]
    indentation: Option<IndentationArgs>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReadMode {
    Slice,
    Indentation,
}

#[derive(Deserialize, Clone)]
struct IndentationArgs {
    #[serde(default)]
    anchor_line: Option<usize>,
    #[serde(default = "defaults::max_levels")]
    max_levels: usize,
    #[serde(default = "defaults::include_siblings")]
    include_siblings: bool,
    #[serde(default = "defaults::include_header")]
    include_header: bool,
    #[serde(default)]
    max_lines: Option<usize>,
}

impl Default for IndentationArgs {
    fn default() -> Self {
        Self {
            anchor_line: None,
            max_levels: defaults::max_levels(),
            include_siblings: defaults::include_siblings(),
            include_header: defaults::include_header(),
            max_lines: None,
        }
    }
}

impl Default for ReadMode {
    fn default() -> Self {
        ReadMode::Slice
    }
}

mod defaults {
    pub fn offset() -> usize {
        1
    }

    pub fn limit() -> usize {
        2000
    }

    pub fn max_levels() -> usize {
        0
    }

    pub fn include_siblings() -> bool {
        false
    }

    pub fn include_header() -> bool {
        true
    }
}

const MAX_LINE_LENGTH: usize = 500;
const TAB_WIDTH: usize = 4;
const COMMENT_PREFIXES: &[&str] = &["#", "//", "--"];

#[derive(Clone, Debug)]
struct LineRecord {
    number: usize,
    raw: String,
    display: String,
    indent: usize,
}

impl LineRecord {
    fn trimmed(&self) -> &str {
        self.raw.trim_start()
    }

    fn is_blank(&self) -> bool {
        self.trimmed().is_empty()
    }

    fn is_comment(&self) -> bool {
        COMMENT_PREFIXES
            .iter()
            .any(|prefix| self.raw.trim().starts_with(prefix))
    }
}

async fn read_slice(
    session_id: &SessionId,
    path: PathBuf,
    offset: usize,
    limit: usize,
) -> Result<Vec<String>, FunctionCallError> {
    let content = fetch_text(session_id, path, Some(offset), Some(limit)).await?;
    let lines = split_lines(&content);
    if lines.is_empty() {
        return Err(FunctionCallError::RespondToModel(
            "offset exceeds file length or file is empty".to_string(),
        ));
    }
    Ok(lines
        .into_iter()
        .enumerate()
        .map(|(idx, line)| format!("L{}: {}", offset + idx, format_line(line.as_bytes())))
        .collect())
}

async fn read_indent_block(
    session_id: &SessionId,
    path: PathBuf,
    offset: usize,
    limit: usize,
    options: IndentationArgs,
) -> Result<Vec<String>, FunctionCallError> {
    let content = fetch_text(session_id, path, None, None).await?;
    if content.is_empty() {
        return Err(FunctionCallError::RespondToModel(
            "file is empty; nothing to read".to_string(),
        ));
    }
    let records = collect_file_lines(&content);
    read_block(records, offset, limit, options)
}

fn collect_file_lines(content: &str) -> Vec<LineRecord> {
    content
        .lines()
        .enumerate()
        .map(|(idx, line)| {
            let raw = line.trim_end_matches('\r').to_string();
            let indent = measure_indent(&raw);
            let display = format_line(raw.as_bytes());
            LineRecord {
                number: idx + 1,
                raw,
                display,
                indent,
            }
        })
        .collect()
}

fn read_block(
    collected: Vec<LineRecord>,
    offset: usize,
    limit: usize,
    options: IndentationArgs,
) -> Result<Vec<String>, FunctionCallError> {
    if offset == 0 {
        return Err(FunctionCallError::RespondToModel(
            "offset must be a 1-indexed line number".to_string(),
        ));
    }

    if collected.is_empty() || offset > collected.len() {
        return Err(FunctionCallError::RespondToModel(
            "offset exceeds file length".to_string(),
        ));
    }

    let anchor_line = options.anchor_line.unwrap_or(offset);
    if anchor_line == 0 || anchor_line > collected.len() {
        return Err(FunctionCallError::RespondToModel(
            "anchor_line exceeds file length".to_string(),
        ));
    }

    let guard_limit = options.max_lines.unwrap_or(limit);
    if guard_limit == 0 {
        return Err(FunctionCallError::RespondToModel(
            "max_lines must be greater than zero".to_string(),
        ));
    }

    let anchor_index = anchor_line - 1;
    let effective_indents = compute_effective_indents(&collected);
    let anchor_indent = effective_indents[anchor_index];

    let min_indent = if options.max_levels == 0 {
        0
    } else {
        anchor_indent.saturating_sub(options.max_levels * TAB_WIDTH)
    };

    let final_limit = limit.min(guard_limit).min(collected.len());
    if final_limit == 1 {
        return Ok(vec![format!(
            "L{}: {}",
            collected[anchor_index].number, collected[anchor_index].display
        )]);
    }

    let mut i: isize = anchor_index as isize - 1;
    let mut j: usize = anchor_index + 1;
    let mut i_counter_min_indent = 0usize;
    let mut j_counter_min_indent = 0usize;

    let mut out = VecDeque::with_capacity(final_limit);
    out.push_back(&collected[anchor_index]);

    while out.len() < final_limit {
        let mut progressed = 0usize;

        if i >= 0 {
            let iu = i as usize;
            if effective_indents[iu] >= min_indent {
                out.push_front(&collected[iu]);
                progressed += 1;
                i -= 1;

                if effective_indents[iu] == min_indent && !options.include_siblings {
                    let allow_header_comment = options.include_header && collected[iu].is_comment();
                    let can_take_line = allow_header_comment || i_counter_min_indent == 0;

                    if can_take_line {
                        i_counter_min_indent += 1;
                    } else {
                        out.pop_front();
                        progressed -= 1;
                        i = -1;
                    }
                }

                if out.len() >= final_limit {
                    break;
                }
            } else {
                i = -1;
            }
        }

        if j < collected.len() {
            let ju = j;
            if effective_indents[ju] >= min_indent {
                out.push_back(&collected[ju]);
                progressed += 1;
                j += 1;

                if effective_indents[ju] == min_indent && !options.include_siblings {
                    if j_counter_min_indent > 0 {
                        out.pop_back();
                        progressed -= 1;
                        j = collected.len();
                    }
                    j_counter_min_indent += 1;
                }
            } else {
                j = collected.len();
            }
        }

        if progressed == 0 {
            break;
        }
    }

    trim_empty_lines(&mut out);

    if let Some(max_lines) = options.max_lines {
        out.truncate(max_lines);
    }

    Ok(out
        .into_iter()
        .map(|record| format!("L{}: {}", record.number, record.display))
        .collect())
}

fn compute_effective_indents(records: &[LineRecord]) -> Vec<usize> {
    let mut effective = Vec::with_capacity(records.len());
    let mut previous_indent = 0usize;
    for record in records {
        if record.is_blank() {
            effective.push(previous_indent);
        } else {
            previous_indent = record.indent;
            effective.push(previous_indent);
        }
    }
    effective
}

fn measure_indent(line: &str) -> usize {
    line.chars()
        .take_while(|c| matches!(c, ' ' | '\t'))
        .map(|c| if c == '\t' { TAB_WIDTH } else { 1 })
        .sum()
}

fn trim_empty_lines(out: &mut VecDeque<&LineRecord>) {
    while matches!(out.front(), Some(line) if line.raw.trim().is_empty()) {
        out.pop_front();
    }
    while matches!(out.back(), Some(line) if line.raw.trim().is_empty()) {
        out.pop_back();
    }
}

fn format_line(bytes: &[u8]) -> String {
    let decoded = String::from_utf8_lossy(bytes);
    if decoded.len() > MAX_LINE_LENGTH {
        take_bytes_at_char_boundary(&decoded, MAX_LINE_LENGTH).to_string()
    } else {
        decoded.into_owned()
    }
}

fn split_lines(content: &str) -> Vec<String> {
    content
        .lines()
        .map(|line| line.trim_end_matches('\r').to_string())
        .collect()
}

async fn fetch_text(
    session_id: &SessionId,
    path: PathBuf,
    line: Option<usize>,
    limit: Option<usize>,
) -> Result<String, FunctionCallError> {
    let request = ReadTextFileRequest {
        session_id: session_id.clone(),
        path,
        line: line.and_then(|value| value.try_into().ok()),
        limit: limit.and_then(|value| value.try_into().ok()),
        meta: None,
    };

    call_client(move |client| async move { client.read_text_file(request).await })
        .await
        .map(|res| res.content)
}

fn session_id_from_conversation_id(id: &ConversationId) -> SessionId {
    SessionId(id.to_string().into())
}

async fn call_client<R, F, Fut>(f: F) -> Result<R, FunctionCallError>
where
    R: Send + 'static,
    F: FnOnce(Arc<AgentSideConnection>) -> Fut + Send + 'static,
    Fut: Future<Output = Result<R, Error>> + 'static,
{
    let client = ACP_CLIENT
        .get()
        .ok_or_else(|| FunctionCallError::Fatal("ACP client not initialized".to_string()))?
        .clone();
    let handle = Handle::current();
    tokio::task::spawn_blocking(move || handle.block_on(async move { f(client).await }))
        .await
        .map_err(|err| FunctionCallError::Fatal(format!("ACP client call failed: {err}")))?
        .map_err(|err| FunctionCallError::RespondToModel(format!("ACP client error: {err}")))
}
