use std::collections::BTreeSet;

use anyhow::Result;

use crate::Outcome;
use crate::manifest::{AddonName, Manifest, Overrides};
use crate::project::Project;
use crate::state::State;
use crate::sync::{Installed, inspect, is_current, resolve_override, same_dir, summarize};

struct Row {
    name: String,
    pin: String,
    state: String,
}

pub(crate) fn status(project: &Project) -> Result<Outcome> {
    let manifest = if project.has_manifest() {
        Manifest::load(&project.manifest_path())?
    } else {
        Manifest::default()
    };
    let overrides = Overrides::load(&project.overrides_path())?;
    let state = State::load(&project.state_dir())?;
    let addons_dir = project.addons_dir();

    let desired: BTreeSet<&AddonName> = manifest
        .addons
        .keys()
        .chain(overrides.addons.keys())
        .collect();
    let mut rows = Vec::new();
    for name in &desired {
        let dir = addons_dir.join(name.as_str());
        let pin = match manifest.addons.get(*name) {
            Some(addon) => {
                let pin = addon.source.short_pin();
                match &addon.version {
                    Some(version) => format!("{version} ({pin})"),
                    None => format!("({pin})"),
                }
            }
            None => "(override only)".to_owned(),
        };
        let installed = inspect(&dir, name, &state);
        let state = match (installed, overrides.addons.get(*name)) {
            (Err(e), _) => format!("error: {e:#}"),
            (Ok(installed), Some(override_dir)) => {
                match resolve_override(project, name, override_dir) {
                    Err(e) => format!("override error: {e:#}"),
                    Ok((source, _)) => match installed {
                        Installed::Link {
                            source: current,
                            owned: true,
                        } if same_dir(&current, &source) => {
                            format!("overridden -> {}", source.display())
                        }
                        Installed::Link { owned: false, .. } | Installed::Unowned => {
                            "not installed by gdget".to_owned()
                        }
                        _ => format!(
                            "override not applied -> {} (run gdget sync)",
                            source.display()
                        ),
                    },
                }
            }
            (Ok(installed), None) => {
                let addon = &manifest.addons[*name];
                match installed {
                    Installed::Missing => "missing (run gdget sync)".to_owned(),
                    Installed::Copy(marker) => {
                        let changes = match marker.changes(&dir) {
                            Ok(changes) if changes.is_empty() => String::new(),
                            Ok(changes) => format!(", modified: {}", summarize(&changes)),
                            Err(e) => format!(", cannot check for changes: {e:#}"),
                        };
                        if is_current(&marker, addon) {
                            format!("installed{changes}")
                        } else {
                            format!(
                                "outdated, {} installed (run gdget sync){changes}",
                                marker.source.short_pin()
                            )
                        }
                    }
                    Installed::Link { owned: true, .. } => {
                        "still linked to an override (run gdget sync)".to_owned()
                    }
                    Installed::Link { owned: false, .. } | Installed::Unowned => {
                        "not installed by gdget".to_owned()
                    }
                }
            }
        };
        rows.push(Row {
            name: name.to_string(),
            pin,
            state,
        });
    }

    if let Ok(entries) = std::fs::read_dir(&addons_dir) {
        let mut extra: Vec<Row> = entries
            .flatten()
            .filter_map(|entry| {
                let file_name = entry.file_name().to_string_lossy().into_owned();
                let owned = match file_name.parse::<AddonName>() {
                    Ok(name) if desired.contains(&name) => return None,
                    Ok(name) => matches!(
                        inspect(&entry.path(), &name, &state),
                        Ok(Installed::Copy(_) | Installed::Link { owned: true, .. })
                    ),
                    Err(_) => false,
                };
                entry.path().is_dir().then(|| Row {
                    name: file_name,
                    pin: "-".to_owned(),
                    state: if owned {
                        "not in addons.toml (sync removes it)".to_owned()
                    } else {
                        "not managed by gdget".to_owned()
                    },
                })
            })
            .collect();
        extra.sort_by(|a, b| a.name.cmp(&b.name));
        rows.extend(extra);
    }

    if rows.is_empty() {
        println!("no addons in addons.toml");
        return Ok(Outcome::Success);
    }
    let name_width = rows.iter().map(|r| r.name.len()).max().unwrap_or(0);
    let pin_width = rows.iter().map(|r| r.pin.len()).max().unwrap_or(0);
    for row in rows {
        println!(
            "{:name_width$}  {:pin_width$}  {}",
            row.name, row.pin, row.state
        );
    }
    Ok(Outcome::Success)
}
