use anyhow::{Context, Result, bail};

use crate::Outcome;
use crate::archive::Archive;
use crate::fetch::{Cache, Fetcher};
use crate::layout;
use crate::manifest::{
    Addon, AddonName, ArchivePath, MANIFEST_FILE, ManifestFile, OVERRIDES_FILE, Overrides, Source,
};
use crate::project::Project;
use crate::report::Reporter;
use crate::state::State;
use crate::sync::{self, Installed, SyncOptions, inspect, label};

/// Pins `url` as addon `name` (re-pinning it if already present) and installs it.
pub(crate) fn add(
    project: &Project,
    name: &AddonName,
    url: &str,
    path: Option<&ArchivePath>,
    version: Option<String>,
    reporter: Reporter,
) -> Result<Outcome> {
    let mut manifest = ManifestFile::open(&project.manifest_path())?;
    let state = State::load(&project.state_dir())?;
    let dir = project.addons_dir().join(name.as_str());
    if let Installed::Unowned | Installed::Link { owned: false, .. } = inspect(&dir, name, &state)?
    {
        bail!(
            "addons/{name} exists but was not installed by gdget. Move or delete it, then \
             run gdget add again"
        );
    }

    if url.starts_with("http://") {
        reporter.warn(format!(
            "{url} is not https: the hash pinned now only proves later downloads match this \
             one, which could have been altered in transit"
        ));
    }
    let fetcher = Fetcher::new(Cache::from_env()?, reporter);
    let (sha256, archive_path) = fetcher.fetch_unpinned(url)?;
    let archive = Archive::open(&archive_path)?;
    let resolved = layout::resolve(archive.tree(), name, path)
        .with_context(|| format!("cannot add `{name}` from {url}"))?;
    if let Some(warning) = &resolved.warning {
        reporter.warn(format!("{name}: {warning}"));
    }

    let addon = Addon {
        version,
        source: Source::Url {
            url: url.to_owned(),
            sha256,
        },
        path: Some(resolved.path),
    };
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
            "{OVERRIDES_FILE} overrides {name}; the pinned version is installed once the \
             override is removed"
        ));
    }
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
    sync::sync(
        project,
        SyncOptions {
            only: Some(name.clone()),
            ..SyncOptions::default()
        },
        reporter,
    )
}
