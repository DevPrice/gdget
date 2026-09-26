use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::manifest::{AddonName, ArchivePath, check_text, check_url};

/// Install pinned Godot addon dependencies from addons.toml.
#[derive(Debug, Parser)]
#[command(version, about)]
pub struct Cli {
    /// Run as if started in this directory.
    #[arg(short = 'C', long = "directory", global = true, value_name = "DIR")]
    pub directory: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Make addons/ match the manifest and local overrides.
    Sync {
        /// Overwrite addons that were modified locally.
        #[arg(long)]
        force: bool,

        /// Report what would change without changing anything; exit 1 on drift.
        #[arg(long, conflicts_with = "force")]
        check: bool,
    },

    /// Download an addon, pin its hash in the manifest, and install it.
    Add {
        /// Install folder name: the addon installs to addons/<NAME>/.
        name: AddonName,

        /// URL of the addon zip.
        #[arg(value_parser = parse_url)]
        url: String,

        /// Folder inside the archive to install, if auto-detection can't decide.
        #[arg(long)]
        path: Option<ArchivePath>,

        /// Display-only version label to record in the manifest.
        #[arg(long = "version-label", value_name = "LABEL", value_parser = parse_text)]
        version_label: Option<String>,
    },

    /// Remove an addon from the manifest and uninstall it.
    Remove {
        /// Install folder name of the addon.
        name: AddonName,
    },

    /// Show each addon's pinned version, installed state, and overrides.
    Status,
}

fn parse_url(url: &str) -> anyhow::Result<String> {
    check_url(url).map(|()| url.to_owned())
}

fn parse_text(text: &str) -> anyhow::Result<String> {
    check_text(text).map(|()| text.to_owned())
}
