use anyhow::{Context, Result, bail};

use crate::Outcome;
use crate::archive::Archive;
use crate::cli::AddonSource;
use crate::fetch::{Cache, Fetcher};
use crate::layout::{self, Tree};
use crate::manifest::{
    Addon, AddonName, ArchivePath, MANIFEST_FILE, ManifestFile, OVERRIDES_FILE, Overrides, Source,
};
use crate::project::Project;
use crate::report::Reporter;
use crate::state::State;
use crate::store::Store;
use crate::sync::{self, Installed, SyncOptions, inspect, label};

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
    let (url, store_version) = match source {
        AddonSource::Url(url) => {
            if url.starts_with("http://") {
                reporter.warn(format!(
                    "{url} is not https: the hash pinned now only proves later downloads \
                     match this one, which could have been altered in transit"
                ));
            }
            (url.clone(), None)
        }
        AddonSource::Store(asset) => {
            let release = Store::from_env()?.resolve(&fetcher, asset)?;
            reporter.action(
                "Found",
                format!("{}/{} {}", asset.publisher, asset.asset, release.version),
            );
            (release.url, Some(release.version))
        }
    };
    let (sha256, archive_path) = fetcher.fetch_unpinned(&url)?;
    let archive = Archive::open(&archive_path)?;
    let name = match name {
        Some(name) => name.clone(),
        None => {
            let name = detect_name(archive.tree(), source)?;
            ensure_managed(project, &name, &state)?;
            name
        }
    };
    let resolved = layout::resolve(archive.tree(), &name, path)
        .with_context(|| format!("cannot add `{name}` from {url}"))?;
    if let Some(warning) = &resolved.warning {
        reporter.warn(format!("{name}: {warning}"));
    }

    let addon = Addon {
        version: version_label.or(store_version),
        source: Source::Url { url, sha256 },
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

fn ensure_managed(project: &Project, name: &AddonName, state: &State) -> Result<()> {
    let dir = project.addons_dir().join(name.as_str());
    if let Installed::Unowned | Installed::Link { owned: false, .. } = inspect(&dir, name, state)? {
        bail!(
            "addons/{name} exists but was not installed by gdget. Move or delete it, then \
             run gdget add again"
        );
    }
    Ok(())
}

/// The archive's only `addons/` folder, else the Asset Store asset's slug. Renaming an
/// addon folder can break its `res://` paths, so a URL source without one needs a NAME.
fn detect_name(tree: &dyn Tree, source: &AddonSource) -> Result<AddonName> {
    let pass_name = "pass the install folder name: gdget add NAME SOURCE";
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
        AddonSource::Url(_) => bail!("the archive has no single addons/ folder; {pass_name}"),
    }
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
