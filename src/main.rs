use anyhow::Result;
use clap::Parser;
use codex_acp::CliArgs;
use codex_arg0::arg0_dispatch_or_else;

fn main() -> Result<()> {
    arg0_dispatch_or_else(|codex_linux_sandbox_exe| async move {
        let cli_args = CliArgs::parse();
        codex_acp::run_main(codex_linux_sandbox_exe, cli_args).await?;
        Ok(())
    })
}
