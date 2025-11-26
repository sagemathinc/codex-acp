# ACP adapter for Codex (used in CoCalc)

**DISCLAIMER:** This repo and the corresponding binaries are *NOT* in any way associated with Zed Industries or OpenAI.  They are instead a product of Sagemath, Inc. and are needed for https://cocalc.com .

This fork exists so we can fully use [Codex](https://github.com/openai/codex) from [ACP-compatible](https://agentclientprotocol.com) clients such as [CoCalc](https://cocalc.com), with resumable sessions, remote code and file execution in a CoCalc project, and file diffs.

## CoCalc Fork-specific changes


The fork includes the following changes from upstream:

- Track our `sagemathinc/codex` fork so we can use the custom tool executor, `ConversationId` plumbing, and overridable tool handlers that are needed for ACP-aware integrations.
- Replace the ad-hoc CLI parsing with `clap`, expose `--session-persist`, `--session-persist=/path`, `--no-session-persist`, and add `--native-shell` for opting into Codex's local sandbox instead of ACP's terminal proxy.
- Introduce an `ACPExecutor` and background command dispatcher that translate Codex shell requests into ACP terminal calls (and fall back to the native executor when requested) so every tool call runs inside the remote ACP session.
- Implement remote `apply_patch` by fetching files over ACP, applying hunks in memory via the exported `codex-apply-patch` helpers, ensuring parent directories exist, and writing the results back through ACP (deleting or moving files via ACP shell commands when necessary).
- Register a remote `read_file` tool handler that streams data through ACP, supports slice and indentation modes, and ensures GPT-5 model families advertise the tool so IDE clients can call it; this replaces the built-in filesystem reader entirely.
- Extend session persistence/logging: CLI overrides feed directly into `SessionStore`, ACP initialization logs the client's filesystem/terminal capabilities, and token usage metadata gets forwarded in ACP notifications with better warning logs on read/write failures.

# Original Readme

Use [Codex](https://github.com/openai/codex) from [ACP-compatible](https://agentclientprotocol.com) clients such as [Zed](https://zed.dev)!

This tool implements an ACP adapter around the Codex CLI, supporting:

- Context @-mentions
- Images
- Tool calls (with permission requests)
- Following
- Edit review
- TODO lists
- Slash commands:
  - /review (with optional instructions)
  - /review-branch
  - /review-commit
  - /init
  - /compact
  - /logout
  - Custom Prompts
- Client MCP servers
- Auth Methods:
  - ChatGPT subscription (requires paid subscription and doesn't work in remote projects)
  - CODEX_API_KEY
  - OPENAI_API_KEY

Learn more about the [Agent Client Protocol](https://agentclientprotocol.com/).

## How to use

### Zed

The latest version of Zed can already use this adapter out of the box.

To use Codex, open the Agent Panel and click "New Codex Thread" from the `+` button menu in the top-right.

Read the docs on [External Agent](https://zed.dev/docs/ai/external-agents) support.

### Other clients

[Submit a PR](https://github.com/zed-industries/codex-acp/pulls) to add yours!

#### Installation

Install the adapter from the latest release for your architecture and OS: https://github.com/zed-industries/codex-acp/releases

You can then use `codex-acp` as a regular ACP agent:

```
OPENAI_API_KEY=sk-... codex-acp
```

Or via npm:

```
npx @zed-industries/codex-acp
```

#### macOS quarantine note

Release binaries are unsigned. If you download on macOS, clear the quarantine bit before first run:

```
xattr -d com.apple.quarantine ./codex-acp
```

### Session persistence

Enable persistent sessions with either:

- `codex-acp --session-persist` (or `--session-persist=/custom/path`)
- Environment variables: `CODEX_SESSION_PERSIST=1` and optionally `CODEX_SESSION_DIR=/custom/path`

By default, metadata lives alongside rollout JSONL files under `${CODEX_HOME}/sessions`. The
manifest keeps track of the rollout path, model/mode overrides, and MCP servers so `/session/load`
can resume a conversation after the agent restarts. Disable persistence with `--no-session-persist`
or `CODEX_SESSION_PERSIST=0` if you want the in-memory behavior back.

## License

Apache-2.0
