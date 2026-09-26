//! Internals of the `gdget` binary, exposed for its integration tests. Not a stable API.

#[cfg(not(any(unix, windows)))]
compile_error!("gdget supports Unix and Windows only");

mod archive;
pub mod cli;
pub mod digest;
mod edit;
pub mod fetch;
mod fsutil;
mod install;
mod layout;
pub mod link;
pub mod manifest;
mod marker;
mod project;
pub mod report;
mod state;
mod status;
mod sync;

use anyhow::Context;

use crate::cli::{Cli, Command};
use crate::project::Project;
use crate::report::Reporter;
use crate::sync::SyncOptions;

/// How a command finished when it did not hit a hard error.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Success,
    /// The command completed but must exit non-zero, e.g. an addon was skipped because
    /// it was modified locally, or `sync --check` found drift. The reason was already
    /// reported.
    Failure,
}

/// Runs one parsed command. Changes the process's working directory when `-C` is given.
pub fn run(cli: Cli, reporter: Reporter) -> anyhow::Result<Outcome> {
    if let Some(dir) = &cli.directory {
        std::env::set_current_dir(dir)
            .with_context(|| format!("cannot change to {}", dir.display()))?;
    }
    let cwd = std::env::current_dir().context("cannot read the current directory")?;
    let project = Project::discover(&cwd)?;
    match cli.command {
        Command::Sync { force, check } => sync::sync(
            &project,
            SyncOptions {
                force,
                check,
                only: None,
            },
            reporter,
        ),
        Command::Add {
            name,
            url,
            path,
            version_label,
        } => edit::add(
            &project,
            &name,
            &url,
            path.as_ref(),
            version_label,
            reporter,
        ),
        Command::Remove { name } => edit::remove(&project, &name, reporter),
        Command::Status => status::status(&project),
    }
}
