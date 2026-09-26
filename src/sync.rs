use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::Outcome;
use crate::archive::Archive;
use crate::fetch::{Cache, Fetcher};
use crate::install::Workspace;
use crate::layout::{self, DirTree};
use crate::link;
use crate::manifest::{Addon, AddonName, Manifest, OVERRIDES_FILE, Overrides, Source};
use crate::marker::{Change, Marker};
use crate::project::Project;
use crate::report::Reporter;
use crate::state::State;

#[derive(Debug, Clone, Default)]
pub struct SyncOptions {
    pub force: bool,
    pub check: bool,
    /// Sync only this addon, leaving every other folder in `addons/` alone.
    pub only: Option<AddonName>,
}

/// The addons that should be present: everything pinned or overridden, narrowed to
/// `only` when set.
fn desired<'a>(
    manifest: &'a Manifest,
    overrides: &'a Overrides,
    only: Option<&AddonName>,
) -> BTreeSet<&'a AddonName> {
    manifest
        .addons
        .keys()
        .chain(overrides.addons.keys())
        .filter(|name| only.is_none_or(|only| only == *name))
        .collect()
}

/// What is at `addons/<name>` right now.
#[derive(Debug)]
pub enum Installed {
    Missing,
    Copy(Marker),
    Link {
        source: PathBuf,
        owned: bool,
    },
    /// A folder or file gdget did not create.
    Unowned,
}

pub fn inspect(dir: &Path, name: &AddonName, state: &State) -> Result<Installed> {
    let Ok(meta) = dir.symlink_metadata() else {
        return Ok(Installed::Missing);
    };
    if meta.file_type().is_symlink() {
        return Ok(Installed::Link {
            source: link::target(dir)?,
            owned: state.links.contains_key(name),
        });
    }
    if !meta.is_dir() {
        return Ok(Installed::Unowned);
    }
    Ok(Marker::read(dir)?.map_or(Installed::Unowned, Installed::Copy))
}

/// Whether an installed copy is exactly what the manifest pins.
pub fn is_current(marker: &Marker, addon: &Addon) -> bool {
    let Source::Url { sha256, .. } = &addon.source;
    marker.sha256 == *sha256 && addon.path.as_ref().is_none_or(|path| *path == marker.path)
}

/// Resolves an override entry to the absolute folder to link.
pub fn resolve_override(
    project: &Project,
    name: &AddonName,
    dir: &Path,
) -> Result<(PathBuf, Option<String>)> {
    let root = project.root().join(dir);
    if !root.is_dir() {
        bail!(
            "the override in {OVERRIDES_FILE} points at {}, which is not a directory",
            root.display()
        );
    }
    let resolved = layout::resolve(&DirTree::new(&root), name, None)
        .with_context(|| format!("cannot use the override at {}", root.display()))?;
    let source = resolved
        .path
        .segments()
        .iter()
        .fold(root, |path, segment| path.join(segment));
    let source = std::path::absolute(&source)
        .with_context(|| format!("cannot resolve {}", source.display()))?;
    Ok((source, resolved.warning))
}

pub fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

#[derive(Debug)]
enum Step {
    UpToDate,
    Install {
        addon: Addon,
        replacing: bool,
    },
    Link {
        source: PathBuf,
    },
    Remove,
    /// Left alone because it has local changes; reported as a warning.
    Modified(Vec<Change>),
    /// Cannot proceed; reported as an error.
    Blocked(String),
}

pub fn sync(project: &Project, options: SyncOptions, reporter: Reporter) -> Result<Outcome> {
    if !project.has_manifest() {
        bail!(
            "no addons.toml in {}; create one with `gdget add <name> <url>`",
            project.root().display()
        );
    }
    if !project.has_godot_project() {
        reporter.warn(format!(
            "{} has addons.toml but no project.godot",
            project.root().display()
        ));
    }
    let manifest = Manifest::load(&project.manifest_path())?;
    let overrides = Overrides::load(&project.overrides_path())?;
    let mut state = State::load(&project.state_dir())?;

    let steps = plan(project, &manifest, &overrides, &state, &options, reporter)?;
    if options.check {
        return Ok(report_check(&steps, reporter));
    }

    let fetcher = Fetcher::new(Cache::from_env()?, reporter);
    let mut archives = BTreeMap::new();
    for (name, step) in &steps {
        if let Step::Install { addon, .. } = step {
            let Source::Url { url, sha256 } = &addon.source;
            let archive = fetcher
                .fetch_pinned(url, sha256)
                .with_context(|| format!("cannot fetch `{name}`"))?;
            archives.insert(name.clone(), archive);
        }
    }

    let addons_dir = project.addons_dir();
    std::fs::create_dir_all(&addons_dir)
        .with_context(|| format!("cannot create {}", addons_dir.display()))?;
    let workspace = Workspace::prepare(&project.state_dir())?;
    let mut failed = false;
    let mut changed = 0;
    for (name, step) in &steps {
        let dir = addons_dir.join(name.as_str());
        let result = match step {
            Step::UpToDate => continue,
            Step::Modified(changes) => {
                reporter.warn(format!(
                    "addons/{name} has local changes and was left as is ({}). Run \
                     `gdget sync --force` to discard them",
                    summarize(changes)
                ));
                failed = true;
                continue;
            }
            Step::Blocked(reason) => {
                reporter.error(format!("{name}: {reason}"));
                failed = true;
                continue;
            }
            Step::Install { addon, replacing } => {
                install_copy(&workspace, &dir, name, addon, &archives[name], reporter).map(|()| {
                    state.links.remove(name);
                    let verb = if *replacing { "Updated" } else { "Installed" };
                    reporter.action(verb, label(name, addon));
                })
            }
            Step::Link { source, .. } => workspace.link_in(source, &dir).map(|()| {
                state.links.insert(name.clone(), source.clone());
                reporter.action("Linked", format!("{name} -> {}", source.display()));
            }),
            Step::Remove => workspace.remove(&dir).map(|()| {
                state.links.remove(name);
                reporter.action("Removed", name);
            }),
        };
        match result {
            Ok(()) => changed += 1,
            Err(e) => {
                reporter.error(format!("{name}: {e:#}"));
                failed = true;
            }
        }
        state.save(&project.state_dir())?;
    }

    let before = state.links.len();
    state
        .links
        .retain(|name, _| link::is_link(&addons_dir.join(name.as_str())));
    if state.links.len() != before {
        state.save(&project.state_dir())?;
    }

    let desired = desired(&manifest, &overrides, options.only.as_ref());
    warn_unignored(project, &desired, reporter);

    let total = desired.len();
    reporter.action(
        "Finished",
        format!(
            "{total} addon{} ({changed} changed, {} up to date)",
            if total == 1 { "" } else { "s" },
            steps
                .values()
                .filter(|step| matches!(step, Step::UpToDate))
                .count()
        ),
    );
    Ok(if failed {
        Outcome::Failure
    } else {
        Outcome::Success
    })
}

fn plan(
    project: &Project,
    manifest: &Manifest,
    overrides: &Overrides,
    state: &State,
    options: &SyncOptions,
    reporter: Reporter,
) -> Result<BTreeMap<AddonName, Step>> {
    let addons_dir = project.addons_dir();
    let force = options.force;
    let mut steps = BTreeMap::new();
    let desired = desired(manifest, overrides, options.only.as_ref());

    for name in &desired {
        let dir = addons_dir.join(name.as_str());
        let installed = match inspect(&dir, name, state) {
            Ok(installed) => installed,
            Err(e) => {
                steps.insert((*name).clone(), Step::Blocked(format!("{e:#}")));
                continue;
            }
        };
        let unowned = || {
            Step::Blocked(format!(
                "addons/{name} exists but was not installed by gdget. Move or delete it, \
                 then run gdget sync again"
            ))
        };
        let step = if let Some(override_dir) = overrides.addons.get(*name) {
            match resolve_override(project, name, override_dir) {
                Err(e) => Step::Blocked(format!("{e:#}")),
                Ok((source, warning)) => {
                    if let Some(warning) = warning {
                        reporter.warn(format!("{name}: {warning}"));
                    }
                    match installed {
                        Installed::Link {
                            source: current,
                            owned: true,
                        } if same_dir(&current, &source) => Step::UpToDate,
                        Installed::Link { owned: true, .. } | Installed::Missing => {
                            Step::Link { source }
                        }
                        Installed::Copy(marker) => guard_changes(
                            &marker,
                            &dir,
                            force,
                            name,
                            reporter,
                            Step::Link { source },
                        ),
                        Installed::Link { owned: false, .. } | Installed::Unowned => unowned(),
                    }
                }
            }
        } else {
            let addon = &manifest.addons[*name];
            let install = |replacing| Step::Install {
                addon: addon.clone(),
                replacing,
            };
            match installed {
                Installed::Copy(marker) if is_current(&marker, addon) => Step::UpToDate,
                Installed::Copy(marker) => {
                    guard_changes(&marker, &dir, force, name, reporter, install(true))
                }
                Installed::Missing => install(false),
                Installed::Link { owned: true, .. } => install(true),
                Installed::Link { owned: false, .. } | Installed::Unowned => unowned(),
            }
        };
        steps.insert((*name).clone(), step);
    }

    let Ok(entries) = std::fs::read_dir(&addons_dir) else {
        return Ok(steps);
    };
    for entry in entries.flatten() {
        let Some(name) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<AddonName>().ok())
        else {
            continue;
        };
        let excluded = options.only.as_ref().is_some_and(|only| *only != name);
        if desired.contains(&name) || excluded {
            continue;
        }
        let dir = entry.path();
        let step = match inspect(&dir, &name, state) {
            Ok(Installed::Copy(marker)) => {
                guard_changes(&marker, &dir, force, &name, reporter, Step::Remove)
            }
            Ok(Installed::Link { owned: true, .. }) => Step::Remove,
            Ok(_) => continue,
            Err(e) => Step::Blocked(format!("{e:#}")),
        };
        steps.insert(name, step);
    }
    Ok(steps)
}

/// Returns `step` unless replacing or removing the copy would lose local changes.
fn guard_changes(
    marker: &Marker,
    dir: &Path,
    force: bool,
    name: &AddonName,
    reporter: Reporter,
    step: Step,
) -> Step {
    match marker.changes(dir) {
        Err(e) => Step::Blocked(format!("{e:#}")),
        Ok(changes) if changes.is_empty() => step,
        Ok(changes) if force => {
            reporter.warn(format!(
                "discarding local changes in addons/{name} ({})",
                summarize(&changes)
            ));
            step
        }
        Ok(changes) => Step::Modified(changes),
    }
}

fn install_copy(
    workspace: &Workspace,
    dir: &Path,
    name: &AddonName,
    addon: &Addon,
    archive_path: &Path,
    reporter: Reporter,
) -> Result<()> {
    let mut archive = Archive::open(archive_path)?;
    let resolved = layout::resolve(archive.tree(), name, addon.path.as_ref())?;
    if let Some(warning) = resolved.warning {
        reporter.warn(format!("{name}: {warning}"));
    }
    let staged = workspace.stage(name.as_str())?;
    let files = archive.extract(&resolved.path, &staged.path())?;
    let Source::Url { url, sha256 } = &addon.source;
    Marker::new(url, *sha256, resolved.path, &files).write(&staged.path())?;
    workspace.swap_in(staged, dir)
}

fn report_check(steps: &BTreeMap<AddonName, Step>, reporter: Reporter) -> Outcome {
    let mut drift = false;
    for (name, step) in steps {
        match step {
            Step::UpToDate => continue,
            Step::Install {
                addon,
                replacing: false,
            } => reporter.action("Missing", label(name, addon)),
            Step::Install {
                addon,
                replacing: true,
            } => reporter.action("Outdated", label(name, addon)),
            Step::Link { source, .. } => {
                reporter.action(
                    "Override",
                    format!("{name} is not linked to {}", source.display()),
                );
            }
            Step::Remove => reporter.action("Extra", name),
            Step::Modified(changes) => {
                reporter.warn(format!(
                    "addons/{name} has local changes ({})",
                    summarize(changes)
                ));
            }
            Step::Blocked(reason) => reporter.error(format!("{name}: {reason}")),
        }
        drift = true;
    }
    if drift {
        Outcome::Failure
    } else {
        reporter.action("Finished", "all addons are up to date");
        Outcome::Success
    }
}

pub fn label(name: &AddonName, addon: &Addon) -> String {
    let Source::Url { sha256, .. } = &addon.source;
    match &addon.version {
        Some(version) => format!("{name} {version} ({})", sha256.short()),
        None => format!("{name} ({})", sha256.short()),
    }
}

pub fn summarize(changes: &[Change]) -> String {
    const SHOWN: usize = 5;
    let mut text = changes
        .iter()
        .take(SHOWN)
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    if changes.len() > SHOWN {
        text.push_str(&format!(", and {} more", changes.len() - SHOWN));
    }
    text
}

/// Warns about installed paths git would commit. Skipped quietly outside a git repo or
/// without git on PATH.
fn warn_unignored(project: &Project, names: &BTreeSet<&AddonName>, reporter: Reporter) {
    let mut paths: Vec<String> = names.iter().map(|name| format!("addons/{name}/")).collect();
    paths.push(".gdget/".to_owned());
    if project.overrides_path().exists() {
        paths.push(OVERRIDES_FILE.to_owned());
    }
    let Ok(output) = std::process::Command::new("git")
        .arg("-C")
        .arg(project.root())
        .arg("check-ignore")
        .args(&paths)
        .output()
    else {
        return;
    };
    // 0: some paths are ignored, 1: none are; anything else means no usable repo.
    if !matches!(output.status.code(), Some(0 | 1)) {
        return;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let ignored: BTreeSet<&str> = stdout.lines().map(str::trim).collect();
    for path in paths.iter().filter(|path| !ignored.contains(path.as_str())) {
        reporter.warn(format!(
            "{path} is not ignored by git; add `/{path}` to .gitignore"
        ));
    }
}
