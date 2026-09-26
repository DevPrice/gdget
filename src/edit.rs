use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::Outcome;
use crate::archive::Archive;
use crate::cli::AddonSource;
use crate::fetch::{Cache, Fetcher};
use crate::fsutil::relative_path;
use crate::git::Git;
use crate::layout::{self, DirTree, Tree};
use crate::manifest::{
    Addon, AddonName, ArchivePath, MANIFEST_FILE, ManifestFile, OVERRIDES_FILE, Overrides,
    OverridesFile, Source, check_text,
};
use crate::project::Project;
use crate::report::Reporter;
use crate::state::State;
use crate::store::Store;
use crate::sync::{self, Installed, SyncOptions, inspect, label, resolve_override};

/// Pins `source` as addon `name` (re-pinning it if already present) and installs it.
/// Without a name, the archive's `addons/` folder name is used.
pub(crate) fn add(
    project: &Project,
    name: Option<&AddonName>,
    source: &AddonSource,
    path: Option<&ArchivePath>,
    version_label: Option<String>,
    reporter: Reporter,
) -> Result<Outcome> {
    let mut manifest = ManifestFile::open(&project.manifest_path())?;
    let state = State::load(&project.state_dir())?;
    if let Some(name) = name {
        ensure_managed(project, name, &state)?;
    }

    let fetcher = Fetcher::new(Cache::from_env()?, reporter);
    let (pinned, archive_path, store_version, _export) = match source {
        AddonSource::Url(url) => {
            if url.starts_with("http://") {
                reporter.warn(format!(
                    "{url} is not https: the hash pinned now only proves later downloads \
                     match this one, which could have been altered in transit"
                ));
            }
            let (sha256, archive) = fetcher.fetch_unpinned(url)?;
            let url = url.clone();
            (Source::Url { url, sha256 }, archive, None, None)
        }
        AddonSource::Store(asset) => {
            let release = Store::from_env()?.resolve(&fetcher, asset)?;
            reporter.action(
                "Found",
                format!("{}/{} {}", asset.publisher, asset.asset, release.version),
            );
            let (sha256, archive) = fetcher.fetch_unpinned(&release.url)?;
            let url = release.url;
            (
                Source::Url { url, sha256 },
                archive,
                Some(release.version),
                None,
            )
        }
        AddonSource::Git { url, reference } => {
            let git = Git::new(fetcher.cache().clone(), reporter);
            let rev = git.resolve(url, reference.as_deref())?;
            let export = git.export(url, &rev)?;
            let source = Source::Git {
                url: url.clone(),
                reference: reference.clone(),
                rev,
            };
            (source, export.to_path_buf(), None, Some(export))
        }
    };
    let archive = Archive::open(&archive_path)?;
    let name = match name {
        Some(name) => name.clone(),
        None => {
            let name = detect_name(archive.tree(), source, path)?;
            ensure_managed(project, &name, &state)?;
            name
        }
    };
    let is_git = matches!(pinned, Source::Git { .. });
    let resolved = layout::resolve(archive.tree(), &name, path, is_git)
        .with_context(|| format!("cannot add `{name}` from {}", pinned.url()))?;
    if let Some(warning) = &resolved.warning {
        reporter.warn(format!("{name}: {warning}"));
    }

    let addon = Addon {
        version: version_label.or(store_version),
        source: pinned,
        path: Some(resolved.path),
    };
    let name = &name;
    manifest.set(name, &addon);
    manifest.save()?;
    reporter.action(
        "Pinned",
        format!("{} in {MANIFEST_FILE}", label(name, &addon)),
    );
    if Overrides::load(&project.overrides_path())?
        .addons
        .contains_key(name)
    {
        reporter.warn(format!(
            "{OVERRIDES_FILE} overrides {name}; the pinned version is installed once you \
             run `gdget unlink {name}`"
        ));
    }
    sync_only(project, name, reporter)
}

fn ensure_managed(project: &Project, name: &AddonName, state: &State) -> Result<()> {
    let dir = project.addons_dir().join(name.as_str());
    if let Installed::Unowned | Installed::Link { owned: false, .. } = inspect(&dir, name, state)? {
        bail!(
            "addons/{name} exists but was not installed by gdget. Move or delete it, then \
             try again"
        );
    }
    Ok(())
}

/// The last folder of `path`, else the source's only `addons/` folder, else the Asset
/// Store asset's slug or the git repository's name. Renaming an addon folder can break
/// its `res://` paths, so a zip URL without any of these needs a NAME.
fn detect_name(
    tree: &dyn Tree,
    source: &AddonSource,
    path: Option<&ArchivePath>,
) -> Result<AddonName> {
    let pass_name = "pass the install folder name: gdget add NAME SOURCE";
    if let Some(folder) = path.and_then(|path| path.segments().last()) {
        return folder
            .parse()
            .with_context(|| format!("cannot install {folder}/ as is; {pass_name}"));
    }
    if let Some(folder) = layout::single_addon_name(tree) {
        return folder
            .parse()
            .with_context(|| format!("cannot install addons/{folder}/ as is; {pass_name}"));
    }
    match source {
        AddonSource::Store(asset) => asset
            .asset
            .parse()
            .with_context(|| format!("cannot name the addon after `{asset}`; {pass_name}")),
        AddonSource::Git { url, .. } => {
            let repo = repo_name(url);
            repo.parse().with_context(|| {
                format!("cannot name the addon after the repository `{repo}`; {pass_name}")
            })
        }
        AddonSource::Url(_) => bail!("the archive has no single addons/ folder; {pass_name}"),
    }
}

/// The last path segment of a repository URL, without `.git`.
fn repo_name(url: &str) -> &str {
    let url = url.trim_end_matches('/');
    let last = url.rsplit(['/', ':']).next().unwrap_or(url);
    last.strip_suffix(".git").unwrap_or(last)
}

/// Overrides addon `name` with the local folder `path` and links it. Without a name, the
/// folder's `addons/` folder name, or its own name if it is an addon folder, is used.
pub(crate) fn link(
    project: &Project,
    name: Option<&AddonName>,
    path: &Path,
    reporter: Reporter,
) -> Result<Outcome> {
    let mut overrides = OverridesFile::open(&project.overrides_path())?;
    let target =
        std::path::absolute(path).with_context(|| format!("cannot resolve {}", path.display()))?;
    if !target.is_dir() {
        bail!("{} is not a directory", path.display());
    }
    let name = match name {
        Some(name) => name.clone(),
        None => detect_local_name(&target)?,
    };
    let dir = override_entry(project.root(), &target)?;

    ensure_managed(project, &name, &State::load(&project.state_dir())?)?;
    resolve_override(project, &name, Path::new(&dir))?;
    overrides.set(&name, &dir);
    overrides.save()?;
    reporter.action("Overrode", format!("{name} with {dir} in {OVERRIDES_FILE}"));
    sync_only(project, &name, reporter)
}

/// Removes the override for `name`, restoring the pinned release if there is one.
pub(crate) fn unlink(project: &Project, name: &AddonName, reporter: Reporter) -> Result<Outcome> {
    let mut overrides = OverridesFile::open(&project.overrides_path())?;
    if !overrides.remove(name) {
        bail!("`{name}` has no override in {OVERRIDES_FILE}");
    }
    overrides.save()?;
    reporter.action(
        "Dropped",
        format!("the override for {name} from {OVERRIDES_FILE}"),
    );
    sync_only(project, name, reporter)
}

fn detect_local_name(dir: &Path) -> Result<AddonName> {
    let tree = DirTree::new(dir);
    let folder = match layout::single_addon_name(&tree) {
        Some(folder) => Some(folder),
        None if layout::is_addon_folder(&tree, &ArchivePath::root()) => tree.root_name(),
        None => None,
    };
    let Some(folder) = folder else {
        bail!(
            "cannot tell which addon {} holds; pass the install folder name: gdget link \
             NAME PATH",
            dir.display()
        );
    };
    folder.parse().with_context(|| {
        format!("cannot install {folder} as is; pass the install folder name: gdget link NAME PATH")
    })
}

/// Relative overrides may climb at most this many folders above the project root.
const MAX_RELATIVE_UPS: usize = 2;

/// How the override for `target` is written: relative to the project root with `/`
/// separators when the two are near each other, as sibling checkouts are, so it survives
/// moving both; otherwise absolute, since a long `../../..` chain breaks on any move.
fn override_entry(root: &Path, target: &Path) -> Result<String> {
    // Both sides are compared canonicalized: the project root comes from the working
    // directory, which the OS reports with symlinks resolved (macOS's /var is
    // /private/var), while PATH is taken as typed.
    let canonical = |path: &Path| {
        path.canonicalize()
            .with_context(|| format!("cannot resolve {}", path.display()))
    };
    let path: PathBuf = relative_path(&canonical(target)?, &canonical(root)?)
        .filter(|path| {
            path.components()
                .filter(|c| *c == Component::ParentDir)
                .count()
                <= MAX_RELATIVE_UPS
        })
        .unwrap_or_else(|| target.to_owned());
    let entry = if path.is_absolute() {
        path.to_str().map(str::to_owned)
    } else if path.as_os_str().is_empty() {
        Some(".".to_owned())
    } else {
        path.components()
            .map(|c| c.as_os_str().to_str())
            .collect::<Option<Vec<_>>>()
            .map(|segments| segments.join("/"))
    };
    let Some(entry) = entry else {
        bail!(
            "the path {} is not valid UTF-8, which {OVERRIDES_FILE} cannot hold",
            target.display()
        );
    };
    check_text(&entry)?;
    Ok(entry)
}

fn sync_only(project: &Project, name: &AddonName, reporter: Reporter) -> Result<Outcome> {
    sync::sync(
        project,
        SyncOptions {
            only: Some(name.clone()),
            ..SyncOptions::default()
        },
        reporter,
    )
}

/// Removes addon `name` from the manifest and uninstalls it.
pub(crate) fn remove(project: &Project, name: &AddonName, reporter: Reporter) -> Result<Outcome> {
    if !project.has_manifest() {
        bail!("no {MANIFEST_FILE} in {}", project.root().display());
    }
    let mut manifest = ManifestFile::open(&project.manifest_path())?;
    if !manifest.remove(name) {
        bail!("`{name}` is not in {MANIFEST_FILE}");
    }
    manifest.save()?;
    reporter.action("Unpinned", format!("{name} from {MANIFEST_FILE}"));
    if Overrides::load(&project.overrides_path())?
        .addons
        .contains_key(name)
    {
        reporter.warn(format!(
            "{OVERRIDES_FILE} still overrides {name}, so it stays linked"
        ));
    }
    sync_only(project, name, reporter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_name_is_the_last_segment_without_dot_git() {
        for (url, name) in [
            (
                "https://github.com/DevPrice/godot-addons.git",
                "godot-addons",
            ),
            ("https://github.com/DevPrice/godot-addons/", "godot-addons"),
            ("ssh://git@host/o/gut.v9.git", "gut.v9"),
            ("git@github.com:DevPrice/message_bus.git", "message_bus"),
            ("git@host:flat.git", "flat"),
        ] {
            assert_eq!(repo_name(url), name, "{url}");
        }
    }
}
