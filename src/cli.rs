use clap::{Args, Parser};
use codex_common::CliConfigOverrides;
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
pub struct CliArgs {
    /// Standard configuration overrides (`-c key=value`)
    #[command(flatten)]
    pub config_overrides: CliConfigOverrides,
    /// Session persistence flags
    #[command(flatten)]
    pub session: SessionPersistCli,
    /// Use Codex's native shell sandbox instead of ACP terminal proxy.
    #[arg(long = "native-shell")]
    pub native_shell: bool,
}

#[derive(Args, Debug, Default, Clone)]
pub struct SessionPersistCli {
    /// Enable session persistence. Optionally provide a directory for manifests.
    #[arg(
        long = "session-persist",
        value_name = "path",
        num_args = 0..=1,
        require_equals = false
    )]
    session_persist: Option<Option<PathBuf>>,
    /// Disable session persistence even if enabled via environment variables.
    #[arg(long = "no-session-persist")]
    no_session_persist: bool,
}

impl SessionPersistCli {
    pub fn flag(&self) -> Option<bool> {
        if self.no_session_persist {
            Some(false)
        } else if self.session_persist.is_some() {
            Some(true)
        } else {
            None
        }
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.session_persist.as_ref().and_then(|opt| opt.clone())
    }
}
