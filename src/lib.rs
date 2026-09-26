pub mod archive;
pub mod cli;
pub mod digest;
pub mod fetch;
pub mod fsutil;
pub mod install;
pub mod layout;
pub mod link;
pub mod manifest;
pub mod marker;
pub mod project;
pub mod report;
pub mod state;

use crate::cli::{Cli, Command};
use crate::report::Reporter;

/// How a command finished when it did not hit a hard error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Success,
    /// The command completed but must exit non-zero, e.g. an addon was skipped because
    /// it was modified locally, or `sync --check` found drift. The reason was already
    /// reported.
    Failure,
}

pub fn run(cli: Cli, _reporter: Reporter) -> anyhow::Result<Outcome> {
    if let Some(dir) = &cli.directory {
        std::env::set_current_dir(dir)
            .map_err(|e| anyhow::anyhow!("cannot change to {}: {e}", dir.display()))?;
    }
    match cli.command {
        Command::Sync { .. } => anyhow::bail!("sync is not implemented yet"),
        Command::Add { .. } => anyhow::bail!("add is not implemented yet"),
        Command::Remove { .. } => anyhow::bail!("remove is not implemented yet"),
        Command::Status => anyhow::bail!("status is not implemented yet"),
    }
}
