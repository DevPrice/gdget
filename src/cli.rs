use std::path::PathBuf;
use std::str::FromStr;

use clap::{Parser, Subcommand};

use crate::manifest::{AddonName, ArchivePath, check_text, check_url};
use crate::store::StoreRef;

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
    #[command(allow_missing_positional = true)]
    Add {
        /// Install folder name: the addon installs to addons/<NAME>/. Defaults to the
        /// archive's addons/ folder name.
        name: Option<AddonName>,

        /// URL of the addon zip, or an Asset Store asset as PUBLISHER/ASSET[@VERSION]
        /// (latest stable release if VERSION is omitted).
        source: AddonSource,

        /// Folder inside the archive to install, if auto-detection can't decide.
        #[arg(long)]
        path: Option<ArchivePath>,

        /// Display-only version label to record in the manifest. Defaults to the Asset
        /// Store release's version.
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

/// Where `add` gets an addon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddonSource {
    Url(String),
    Store(StoreRef),
}

impl FromStr for AddonSource {
    type Err = anyhow::Error;

    fn from_str(source: &str) -> anyhow::Result<Self> {
        if source.contains("://") {
            check_url(source)?;
            Ok(Self::Url(source.to_owned()))
        } else {
            source.parse().map(Self::Store)
        }
    }
}

fn parse_text(text: &str) -> anyhow::Result<String> {
    check_text(text).map(|()| text.to_owned())
}
