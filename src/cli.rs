use std::ffi::OsString;
use std::path::PathBuf;
use std::str::FromStr;

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser, Subcommand};

use crate::git::{check_git_url, check_ref, is_scp_like};
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
        /// last folder of --path, else the source's addons/ folder name, else the Asset
        /// Store asset or git repository name.
        name: Option<AddonName>,

        /// URL of the addon zip or git repository, or an Asset Store asset as
        /// PUBLISHER/ASSET[@VERSION] (latest stable release if VERSION is omitted). URLs
        /// ending in .git and SSH addresses are git repositories.
        source: AddonSource,

        /// Treat SOURCE as a git repository even though it doesn't end in .git.
        #[arg(long)]
        git: bool,

        /// Branch, tag or full commit hash to pin from a git repository. Defaults to the
        /// repository's default branch.
        #[arg(long = "ref", value_name = "REF", value_parser = parse_ref)]
        reference: Option<String>,

        /// Folder inside the archive or repository to install, if auto-detection can't
        /// decide.
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

    /// Use a local folder in place of an addon's pinned release.
    #[command(allow_missing_positional = true)]
    Link {
        /// Install folder name: addons/<NAME>/ links to PATH. Defaults to the folder's
        /// addons/ folder name, or its own name if it is an addon folder.
        name: Option<AddonName>,

        /// The local build: the addon folder itself, or a folder laid out like the
        /// release zip.
        path: PathBuf,
    },

    /// Stop using a local folder and go back to the pinned release.
    Unlink {
        /// Install folder name of the addon.
        name: AddonName,
    },

    /// Show each addon's pinned version, installed state, and overrides.
    Status,
}

impl Cli {
    /// Parses the process's arguments, exiting with status 2 on a usage error.
    pub fn parse_args() -> Self {
        Self::try_parse_args_from(std::env::args_os()).unwrap_or_else(|e| e.exit())
    }

    /// Parses `args`, including the checks clap can't express, such as `--ref` needing a
    /// git source.
    pub fn try_parse_args_from<I, T>(args: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let mut cli = Self::try_parse_from(args)?;
        if let Command::Add {
            source,
            git,
            reference,
            ..
        } = &mut cli.command
        {
            apply_git_flags(source, *git, reference.take())
                .map_err(|message| Self::command().error(ErrorKind::ArgumentConflict, message))?;
        }
        Ok(cli)
    }
}

fn apply_git_flags(
    source: &mut AddonSource,
    git: bool,
    reference: Option<String>,
) -> Result<(), String> {
    if git {
        match source {
            AddonSource::Url(url) => {
                check_git_url(url).map_err(|e| format!("{e:#}"))?;
                *source = AddonSource::Git {
                    url: std::mem::take(url),
                    reference: None,
                };
            }
            AddonSource::Git { .. } => {}
            AddonSource::Store(_) => return Err("--git needs a repository URL as SOURCE".into()),
        }
    }
    match (source, reference) {
        (
            AddonSource::Git {
                reference: slot, ..
            },
            reference,
        ) => *slot = reference,
        (_, Some(_)) => {
            return Err(
                "--ref applies only to git repositories; add --git if SOURCE is one".into(),
            );
        }
        (_, None) => {}
    }
    Ok(())
}

/// Where `add` gets an addon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddonSource {
    Url(String),
    Store(StoreRef),
    Git {
        url: String,
        /// `None` means the repository's default branch.
        reference: Option<String>,
    },
}

impl FromStr for AddonSource {
    type Err = anyhow::Error;

    fn from_str(source: &str) -> anyhow::Result<Self> {
        let is_git = (source.contains("://") && source.trim_end_matches('/').ends_with(".git"))
            || source.starts_with("ssh://")
            || is_scp_like(source);
        if is_git {
            check_git_url(source)?;
            Ok(Self::Git {
                url: source.to_owned(),
                reference: None,
            })
        } else if source.contains("://") {
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

fn parse_ref(reference: &str) -> anyhow::Result<String> {
    check_ref(reference).map(|()| reference.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add_source(args: &[&str]) -> Result<AddonSource, clap::Error> {
        let cli = Cli::try_parse_args_from(["gdget", "add"].iter().chain(args))?;
        match cli.command {
            Command::Add { source, .. } => Ok(source),
            _ => unreachable!(),
        }
    }

    fn git(url: &str, reference: Option<&str>) -> AddonSource {
        AddonSource::Git {
            url: url.to_owned(),
            reference: reference.map(str::to_owned),
        }
    }

    #[test]
    fn repository_urls_and_ssh_addresses_are_git() {
        for url in [
            "https://github.com/DevPrice/godot-addons.git",
            "https://github.com/DevPrice/godot-addons.git/",
            "ssh://git@github.com/DevPrice/godot-addons",
            "git@github.com:DevPrice/godot-addons.git",
        ] {
            assert_eq!(add_source(&[url]).unwrap(), git(url, None), "{url}");
        }
        assert_eq!(
            add_source(&["https://x/a.zip"]).unwrap(),
            AddonSource::Url("https://x/a.zip".into())
        );
        assert!(matches!(
            add_source(&["devprice/godot-slang@v6"]).unwrap(),
            AddonSource::Store(_)
        ));
    }

    #[test]
    fn git_flag_and_ref_apply_to_repositories() {
        let url = "https://github.com/DevPrice/godot-addons";
        assert_eq!(add_source(&[url, "--git"]).unwrap(), git(url, None));
        assert_eq!(
            add_source(&[url, "--git", "--ref", "v1"]).unwrap(),
            git(url, Some("v1"))
        );
        let url = "git@github.com:o/r.git";
        assert_eq!(
            add_source(&[url, "--ref", "main"]).unwrap(),
            git(url, Some("main"))
        );
    }

    #[test]
    fn git_flags_without_a_repository_are_usage_errors() {
        for args in [
            &["https://x/a.zip", "--ref", "main"][..],
            &["devprice/godot-slang", "--git"],
            &["http://x/r", "--git"],
            &["https://x/r.git", "--ref", "--upload-pack=x"],
            &["http://x/r.git"],
        ] {
            let err = add_source(args).unwrap_err();
            assert_eq!(err.exit_code(), 2, "{args:?}: {err}");
        }
    }
}
