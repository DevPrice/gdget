use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use toml_edit::{DocumentMut, Item, Table, value};

use crate::archive::ExtractedFile;
use crate::digest::Sha256;
use crate::manifest::ArchivePath;

/// Written into every addon folder gdget installs; its presence is what makes the folder
/// gdget's to replace or remove.
pub const MARKER_FILE: &str = ".gdget.toml";

const FORMAT: i64 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    pub url: String,
    pub sha256: Sha256,
    /// The archive folder that was installed, after layout resolution.
    pub path: ArchivePath,
    /// Every installed file, `/`-separated and relative to the addon folder.
    pub files: BTreeMap<String, Sha256>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Change {
    Modified(String),
    Added(String),
    Removed(String),
}

impl std::fmt::Display for Change {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Change::Modified(path) => write!(f, "modified: {path}"),
            Change::Added(path) => write!(f, "added: {path}"),
            Change::Removed(path) => write!(f, "removed: {path}"),
        }
    }
}

impl Marker {
    pub fn new(url: &str, sha256: Sha256, path: ArchivePath, files: &[ExtractedFile]) -> Self {
        Self {
            url: url.to_owned(),
            sha256,
            path,
            files: files
                .iter()
                .map(|file| (file.path.clone(), file.sha256))
                .collect(),
        }
    }

    /// Reads the marker in `addon_dir`; `None` means gdget did not install this folder.
    pub fn read(addon_dir: &Path) -> Result<Option<Self>> {
        let path = addon_dir.join(MARKER_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
        };
        Self::parse(&text)
            .map(Some)
            .with_context(|| format!("invalid install marker {}", path.display()))
    }

    fn parse(text: &str) -> Result<Self> {
        let doc: DocumentMut = text.parse()?;
        let format = doc.get("format").and_then(Item::as_integer);
        if format != Some(FORMAT) {
            bail!("unsupported marker format {format:?}; it may need a newer version of gdget");
        }
        let string = |key: &str| {
            doc.get(key)
                .and_then(Item::as_str)
                .ok_or_else(|| anyhow!("missing `{key}`"))
        };
        let files = doc
            .get("files")
            .and_then(Item::as_table_like)
            .ok_or_else(|| anyhow!("missing `files`"))?
            .iter()
            .map(|(path, hash)| {
                let hash = hash
                    .as_str()
                    .ok_or_else(|| anyhow!("`files.{path}` must be a string"))?;
                Ok((path.to_owned(), hash.parse()?))
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            url: string("url")?.to_owned(),
            sha256: string("sha256")?.parse()?,
            path: string("path")?.parse()?,
            files,
        })
    }

    pub fn write(&self, addon_dir: &Path) -> Result<()> {
        let mut doc = DocumentMut::new();
        doc.decor_mut()
            .set_prefix("# Written by gdget to track this install. Do not edit.\n");
        doc["format"] = value(FORMAT);
        doc["url"] = value(&self.url);
        doc["sha256"] = value(self.sha256.to_string());
        doc["path"] = value(self.path.to_string());
        let mut files = Table::new();
        for (path, hash) in &self.files {
            files.insert(path, value(hash.to_string()));
        }
        doc["files"] = Item::Table(files);
        let path = addon_dir.join(MARKER_FILE);
        std::fs::write(&path, doc.to_string())
            .with_context(|| format!("cannot write {}", path.display()))
    }

    /// Compares `addon_dir` with what was installed, ignoring files Godot and the OS
    /// generate there.
    pub fn changes(&self, addon_dir: &Path) -> Result<Vec<Change>> {
        let mut on_disk = BTreeMap::new();
        collect_files(addon_dir, "", &mut on_disk)?;
        let mut changes = Vec::new();
        for (path, expected) in &self.files {
            if is_generated(path) {
                continue;
            }
            match on_disk.remove(path) {
                None => changes.push(Change::Removed(path.clone())),
                Some(file) => {
                    let actual = Sha256::of_file(&file).ok();
                    if actual != Some(*expected) {
                        changes.push(Change::Modified(path.clone()));
                    }
                }
            }
        }
        changes.extend(
            on_disk
                .into_keys()
                .filter(|path| path != MARKER_FILE && !is_generated(path))
                .map(Change::Added),
        );
        changes.sort();
        Ok(changes)
    }
}

/// Files that appear in an addon folder without anyone editing it: Godot's `.uid` and
/// `.import` sidecars, the `~`-prefixed copies Godot makes of GDExtension libraries on
/// Windows so they can be hot-reloaded, and OS litter.
fn is_generated(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.ends_with(".uid")
        || name.ends_with(".import")
        || name.starts_with('~')
        || name == ".DS_Store"
        || name == "Thumbs.db"
}

fn collect_files(
    dir: &Path,
    prefix: &str,
    out: &mut BTreeMap<String, std::path::PathBuf>,
) -> Result<()> {
    let entries =
        std::fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("cannot read {}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let relative = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if entry.file_type()?.is_dir() {
            collect_files(&entry.path(), &relative, out)?;
        } else {
            out.insert(relative, entry.path());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install(dir: &Path, files: &[(&str, &str)]) -> Marker {
        let mut extracted = Vec::new();
        for (path, contents) in files {
            let full = dir.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(&full, contents).unwrap();
            extracted.push(ExtractedFile {
                path: (*path).to_owned(),
                sha256: Sha256::of_bytes(contents.as_bytes()),
            });
        }
        let marker = Marker::new(
            "https://example.com/a.zip",
            Sha256::of_bytes(b"archive"),
            "addons/a".parse().unwrap(),
            &extracted,
        );
        marker.write(dir).unwrap();
        marker
    }

    #[test]
    fn round_trips_through_the_file() {
        let temp = tempfile::tempdir().unwrap();
        let marker = install(temp.path(), &[("plugin.cfg", "x"), ("bin/a b.dll", "y")]);
        assert_eq!(Marker::read(temp.path()).unwrap(), Some(marker));
    }

    #[test]
    fn missing_marker_means_not_owned() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(Marker::read(temp.path()).unwrap(), None);
    }

    #[test]
    fn corrupt_marker_is_an_error() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join(MARKER_FILE), "format = 99").unwrap();
        let err = format!("{:#}", Marker::read(temp.path()).unwrap_err());
        assert!(err.contains("newer version of gdget"), "{err}");
    }

    #[test]
    fn clean_install_has_no_changes_even_after_godot_touches_it() {
        let temp = tempfile::tempdir().unwrap();
        let marker = install(
            temp.path(),
            &[
                ("plugin.cfg", "x"),
                ("a.gd", "code"),
                ("a.gd.uid", "uid://1"),
            ],
        );
        std::fs::write(temp.path().join("a.gd.uid"), "uid://regenerated").unwrap();
        std::fs::write(temp.path().join("icon.png.import"), "[remap]").unwrap();
        std::fs::write(temp.path().join("~lib.dll"), "hot reload copy").unwrap();
        assert_eq!(marker.changes(temp.path()).unwrap(), vec![]);
    }

    #[test]
    fn detects_modified_added_and_removed_files() {
        let temp = tempfile::tempdir().unwrap();
        let marker = install(
            temp.path(),
            &[("plugin.cfg", "x"), ("a.gd", "code"), ("sub/b.gd", "b")],
        );
        std::fs::write(temp.path().join("a.gd"), "edited").unwrap();
        std::fs::remove_file(temp.path().join("sub").join("b.gd")).unwrap();
        std::fs::write(temp.path().join("sub").join("new.gd"), "new").unwrap();
        assert_eq!(
            marker.changes(temp.path()).unwrap(),
            vec![
                Change::Modified("a.gd".into()),
                Change::Added("sub/new.gd".into()),
                Change::Removed("sub/b.gd".into()),
            ]
        );
    }
}
