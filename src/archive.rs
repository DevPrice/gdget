use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use zip::ZipArchive;

use crate::digest::{Sha256, copy_hashed};
use crate::layout::EntryTree;
use crate::manifest::{ArchivePath, is_reserved_on_windows};

/// A file written by [`Archive::extract`], relative to the destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedFile {
    /// `/`-separated path relative to the installed addon folder.
    pub path: String,
    pub sha256: Sha256,
}

pub struct Archive {
    path: PathBuf,
    zip: ZipArchive<File>,
    /// Validated path segments of each entry, indexed like the zip's entries.
    entries: Vec<Vec<String>>,
    tree: EntryTree,
}

impl Archive {
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        let mut zip = ZipArchive::new(file)
            .with_context(|| format!("{} is not a valid zip archive", path.display()))?;
        let mut entries = Vec::with_capacity(zip.len());
        let mut tree = EntryTree::default();
        for index in 0..zip.len() {
            let entry = zip.by_index_raw(index)?;
            let segments = entry_segments(entry.name())
                .with_context(|| format!("unsafe entry in {}", path.display()))?;
            if entry.is_dir() {
                tree.add_dir(&segments);
            } else {
                tree.add_file(&segments);
            }
            entries.push(segments);
        }
        Ok(Self {
            path: path.to_owned(),
            zip,
            entries,
            tree,
        })
    }

    pub fn tree(&self) -> &EntryTree {
        &self.tree
    }

    /// Extracts the contents of the `prefix` folder into `dest`, which must not exist yet.
    pub fn extract(&mut self, prefix: &ArchivePath, dest: &Path) -> Result<Vec<ExtractedFile>> {
        std::fs::create_dir(dest).with_context(|| format!("cannot create {}", dest.display()))?;
        let mut extracted = Vec::new();
        for index in 0..self.entries.len() {
            let Some(relative) = self.entries[index].strip_prefix(prefix.segments()) else {
                continue;
            };
            if relative.is_empty() {
                continue;
            }
            check_portable(relative).with_context(|| {
                format!(
                    "cannot extract `{}` from {}",
                    relative.join("/"),
                    self.path.display()
                )
            })?;
            let out = relative
                .iter()
                .fold(dest.to_owned(), |path, s| path.join(s));
            let relative = relative.join("/");
            let mut entry = self.zip.by_index(index)?;
            if entry.is_dir() {
                std::fs::create_dir_all(&out)
                    .with_context(|| format!("cannot create {}", out.display()))?;
                continue;
            }
            // Symlinks would need Developer Mode on Windows and could point outside the
            // install; addons that need them (e.g. deep macOS frameworks) must flatten them.
            if entry.is_symlink() {
                bail!(
                    "{} contains a symbolic link ({relative}), which gdget does not extract",
                    self.path.display()
                );
            }
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("cannot create {}", parent.display()))?;
            }
            let mut file = File::create_new(&out)
                .with_context(|| format!("cannot create {}", out.display()))?;
            let (_, sha256) = copy_hashed(&mut entry, &mut file).with_context(|| {
                format!("cannot extract {relative} from {}", self.path.display())
            })?;
            #[cfg(unix)]
            if let Some(mode) = entry.unix_mode() {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode & 0o777))
                    .with_context(|| format!("cannot set permissions on {}", out.display()))?;
            }
            extracted.push(ExtractedFile {
                path: relative,
                sha256,
            });
        }
        Ok(extracted)
    }
}

/// Splits a zip entry name into path segments, rejecting anything that could escape the
/// destination. Control characters are refused too, since names end up in log output.
fn entry_segments(name: &str) -> Result<Vec<String>> {
    if name.chars().any(char::is_control) {
        bail!("{name:?} contains control characters");
    }
    let normalized = name.replace('\\', "/");
    if normalized.starts_with('/') || normalized.contains(':') {
        bail!("`{name}` is an absolute path");
    }
    let mut segments = Vec::new();
    for segment in normalized.split('/') {
        match segment {
            "" | "." => {}
            ".." => bail!("`{name}` points outside the archive"),
            _ => segments.push(segment.to_owned()),
        }
    }
    if segments.is_empty() {
        bail!("`{name}` is an empty path");
    }
    Ok(segments)
}

/// Refuses names Windows would silently change or treat as devices: trailing dots and
/// spaces are stripped (so `a.` and `a` collide), and `CON`, `NUL`, `COM1`... open devices
/// on Windows 10. Applied everywhere so an addon installs the same on every platform.
fn check_portable(relative: &[String]) -> Result<()> {
    for segment in relative {
        if segment.ends_with('.') || segment.ends_with(' ') {
            bail!("`{segment}` ends with a dot or space, which Windows strips");
        }
        if segment.contains(['<', '>', '"', '|', '?', '*']) {
            bail!("`{segment}` contains a character Windows does not allow in file names");
        }
        if is_reserved_on_windows(segment) {
            bail!("`{segment}` is a reserved device name on Windows");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use zip::write::SimpleFileOptions;

    use super::*;
    use crate::layout::Tree;

    enum Item<'a> {
        File(&'a str, &'a [u8]),
        Dir(&'a str),
        Symlink(&'a str, &'a str),
    }

    fn write_zip(dir: &Path, items: &[Item]) -> PathBuf {
        let path = dir.join("fixture.zip");
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        let options = SimpleFileOptions::default().unix_permissions(0o755);
        for item in items {
            match item {
                Item::File(name, data) => {
                    zip.start_file(*name, options).unwrap();
                    zip.write_all(data).unwrap();
                }
                Item::Dir(name) => zip.add_directory(*name, options).unwrap(),
                Item::Symlink(name, target) => zip.add_symlink(*name, *target, options).unwrap(),
            }
        }
        zip.finish().unwrap();
        path
    }

    #[test]
    fn extracts_only_the_prefix_and_hashes_files() {
        let temp = tempfile::tempdir().unwrap();
        let zip = write_zip(
            temp.path(),
            &[
                Item::File("README.md", b"readme"),
                Item::Dir("addons/slang/empty/"),
                Item::File("addons/slang/plugin.cfg", b"[plugin]"),
                Item::File("addons/slang/bin/lib.so", b"\x7fELF"),
            ],
        );
        let mut archive = Archive::open(&zip).unwrap();
        assert!(archive.tree().is_dir(&["addons".into(), "slang".into()]));

        let dest = temp.path().join("out");
        let mut files = archive
            .extract(&"addons/slang".parse().unwrap(), &dest)
            .unwrap();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(
            files,
            vec![
                ExtractedFile {
                    path: "bin/lib.so".into(),
                    sha256: Sha256::of_bytes(b"\x7fELF"),
                },
                ExtractedFile {
                    path: "plugin.cfg".into(),
                    sha256: Sha256::of_bytes(b"[plugin]"),
                },
            ]
        );
        assert_eq!(std::fs::read(dest.join("plugin.cfg")).unwrap(), b"[plugin]");
        assert!(dest.join("empty").is_dir());
        assert!(!dest.join("README.md").exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dest.join("bin/lib.so"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o755);
        }
    }

    #[test]
    fn extracts_whole_archive_for_root_prefix() {
        let temp = tempfile::tempdir().unwrap();
        let zip = write_zip(temp.path(), &[Item::File("plugin.cfg", b"x")]);
        let dest = temp.path().join("out");
        let files = Archive::open(&zip)
            .unwrap()
            .extract(&ArchivePath::root(), &dest)
            .unwrap();
        assert_eq!(files.len(), 1);
        assert!(dest.join("plugin.cfg").is_file());
    }

    #[test]
    fn rejects_entries_that_escape() {
        for name in [
            "../evil.gd",
            "a/../../evil.gd",
            "/abs.gd",
            "C:/win.gd",
            "a\\..\\..\\b",
            "a/\u{1b}]8;;x\u{7}.gd",
            "a/line\nbreak.gd",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let zip = write_zip(temp.path(), &[Item::File(name, b"x")]);
            let err = Archive::open(&zip).err().expect(name);
            assert!(
                format!("{err:#}").contains("unsafe entry"),
                "{name}: {err:#}"
            );
        }
    }

    #[test]
    fn rejects_names_windows_would_alter_only_where_extracted() {
        for name in ["a/c./d.gd", "a/trail /x.gd", "a/CON", "a/aux.gd", "a/q?.gd"] {
            let temp = tempfile::tempdir().unwrap();
            let zip = write_zip(
                temp.path(),
                &[Item::File(name, b"x"), Item::File("b/COM1.txt", b"x")],
            );
            let mut archive = Archive::open(&zip).expect(name);
            let dest = temp.path().join("out");
            let err = archive
                .extract(&"a".parse().unwrap(), &dest)
                .expect_err(name);
            assert!(format!("{err:#}").contains("Windows"), "{name}: {err:#}");
        }

        let temp = tempfile::tempdir().unwrap();
        let zip = write_zip(
            temp.path(),
            &[Item::File("a/ok.gd", b"x"), Item::File("docs/aux.md", b"x")],
        );
        Archive::open(&zip)
            .unwrap()
            .extract(&"a".parse().unwrap(), &temp.path().join("out"))
            .unwrap();
    }

    #[test]
    fn rejects_symlinks_in_the_installed_folder() {
        let temp = tempfile::tempdir().unwrap();
        let zip = write_zip(
            temp.path(),
            &[
                Item::File("addons/a/plugin.cfg", b"x"),
                Item::Symlink("addons/a/link", "../../outside"),
            ],
        );
        let err = Archive::open(&zip)
            .unwrap()
            .extract(&"addons/a".parse().unwrap(), &temp.path().join("out"))
            .unwrap_err();
        assert!(err.to_string().contains("symbolic link (link)"), "{err}");
    }

    #[test]
    fn rejects_non_zip_input() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("page.zip");
        std::fs::write(&path, "<html>not found</html>").unwrap();
        let err = Archive::open(&path).err().unwrap();
        assert!(
            err.to_string().contains("is not a valid zip archive"),
            "{err}"
        );
    }
}
