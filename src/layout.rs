use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::manifest::{AddonName, ArchivePath};

/// Root entries that archivers add and that never hold addon content.
const IGNORED_ROOT_ENTRIES: &[&str] = &["__MACOSX"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
}

/// A read-only view of an archive's or directory's folder structure.
pub trait Tree {
    /// Entries directly inside `dir`, sorted by name; empty if `dir` does not exist.
    fn children(&self, dir: &[String]) -> Vec<Entry>;

    fn is_dir(&self, dir: &[String]) -> bool;

    /// The name of the root folder itself, when it has one (a directory, not an archive).
    fn root_name(&self) -> Option<String> {
        None
    }
}

/// An in-memory tree built from archive entry paths.
#[derive(Debug, Default, Clone)]
pub struct EntryTree {
    dirs: BTreeSet<Vec<String>>,
    files: BTreeSet<Vec<String>>,
}

impl EntryTree {
    pub fn add_file(&mut self, path: &[String]) {
        if let Some((_, parent)) = path.split_last() {
            self.add_dir(parent);
            self.files.insert(path.to_vec());
        }
    }

    pub fn add_dir(&mut self, path: &[String]) {
        for len in 1..=path.len() {
            self.dirs.insert(path[..len].to_vec());
        }
    }
}

impl Tree for EntryTree {
    fn children(&self, dir: &[String]) -> Vec<Entry> {
        let child_of = |path: &Vec<String>| path.len() == dir.len() + 1 && path.starts_with(dir);
        let mut entries: Vec<Entry> = self
            .dirs
            .iter()
            .filter(|p| child_of(p))
            .map(|p| Entry {
                name: p[dir.len()].clone(),
                is_dir: true,
            })
            .chain(self.files.iter().filter(|p| child_of(p)).map(|p| Entry {
                name: p[dir.len()].clone(),
                is_dir: false,
            }))
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        entries
    }

    fn is_dir(&self, dir: &[String]) -> bool {
        dir.is_empty() || self.dirs.contains(dir)
    }
}

/// A tree over a directory on disk, used for local overrides.
#[derive(Debug, Clone)]
pub struct DirTree {
    root: PathBuf,
}

impl DirTree {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_owned(),
        }
    }

    fn path(&self, dir: &[String]) -> PathBuf {
        dir.iter().fold(self.root.clone(), |path, s| path.join(s))
    }
}

impl Tree for DirTree {
    fn children(&self, dir: &[String]) -> Vec<Entry> {
        let Ok(read_dir) = std::fs::read_dir(self.path(dir)) else {
            return Vec::new();
        };
        let mut entries: Vec<Entry> = read_dir
            .filter_map(Result::ok)
            .filter_map(|e| {
                Some(Entry {
                    name: e.file_name().into_string().ok()?,
                    is_dir: e.path().is_dir(),
                })
            })
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        entries
    }

    fn is_dir(&self, dir: &[String]) -> bool {
        self.path(dir).is_dir()
    }

    fn root_name(&self) -> Option<String> {
        self.root
            .canonicalize()
            .ok()?
            .file_name()?
            .to_str()
            .map(str::to_owned)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub path: ArchivePath,
    pub warning: Option<String>,
}

/// Finds the folder in `tree` to install as `addons/<name>/`.
///
/// Rules, first match wins:
/// 1. `explicit` (the manifest's `path`), which must exist.
/// 2. `addons/<name>/`, at the root or inside a single wrapper folder.
/// 3. The root, then the wrapper folder, if it is an addon folder (holds `plugin.cfg` or a
///    `.gdextension` file) or, for a directory, is itself named `<name>`.
/// 4. The only folder under `addons/`, with a warning because it gets renamed.
///
/// Anything else is an error that lists what was found and asks for `path`.
pub fn resolve(
    tree: &dyn Tree,
    name: &AddonName,
    explicit: Option<&ArchivePath>,
) -> Result<Resolved> {
    if let Some(path) = explicit {
        if !tree.is_dir(path.segments()) {
            bail!("`path = \"{path}\"` is not a folder in the source");
        }
        return Ok(found(path.clone()));
    }

    let root = ArchivePath::root();
    let bases: Vec<ArchivePath> = std::iter::once(root.clone())
        .chain(wrapper_folder(tree))
        .collect();

    for base in &bases {
        let candidate = base.join("addons").join(name.as_str());
        if tree.is_dir(candidate.segments()) {
            return Ok(found(candidate));
        }
    }

    for base in &bases {
        if is_addon_folder(tree, base) {
            return Ok(found(base.clone()));
        }
    }
    if tree.root_name().as_deref() == Some(name.as_str()) {
        return Ok(found(root));
    }

    let others: Vec<ArchivePath> = bases
        .iter()
        .map(|base| base.join("addons"))
        .flat_map(|addons| {
            tree.children(addons.segments())
                .into_iter()
                .filter(|e| e.is_dir)
                .map(move |e| addons.join(&e.name))
        })
        .collect();
    match others.as_slice() {
        [only] => Ok(Resolved {
            path: only.clone(),
            warning: Some(format!(
                "the source has no addons/{name}/; installing {only}/ as addons/{name}/. \
                 res:// paths inside it (such as library paths in .gdextension files) may \
                 expect the original folder name"
            )),
        }),
        [] => {
            let top_level = tree
                .children(&[])
                .iter()
                .map(|e| {
                    if e.is_dir {
                        format!("{}/", e.name)
                    } else {
                        e.name.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "cannot find the addon folder for `{name}`: expected addons/{name}/ or a \
                 folder with plugin.cfg or a .gdextension file, but the top level has: \
                 {top_level}. Set `path` to the folder to install"
            )
        }
        many => bail!(
            "cannot tell which folder to install as `{name}`: found {}. Set `path` to the \
             folder to install",
            many.iter()
                .map(|p| format!("{p}/"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn found(path: ArchivePath) -> Resolved {
    Resolved {
        path,
        warning: None,
    }
}

/// The root's only entry, when it is a folder (GitHub source zips wrap `<repo>-<ref>/`).
fn wrapper_folder(tree: &dyn Tree) -> Option<ArchivePath> {
    let entries: Vec<Entry> = tree
        .children(&[])
        .into_iter()
        .filter(|e| !IGNORED_ROOT_ENTRIES.contains(&e.name.as_str()))
        .collect();
    match entries.as_slice() {
        [only] if only.is_dir => Some(ArchivePath::root().join(&only.name)),
        _ => None,
    }
}

fn is_addon_folder(tree: &dyn Tree, dir: &ArchivePath) -> bool {
    tree.children(dir.segments())
        .iter()
        .any(|e| !e.is_dir && (e.name == "plugin.cfg" || e.name.ends_with(".gdextension")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(files: &[&str]) -> EntryTree {
        let mut tree = EntryTree::default();
        for file in files {
            let segments: Vec<String> = file.split('/').map(str::to_owned).collect();
            if file.ends_with('/') {
                tree.add_dir(&segments[..segments.len() - 1]);
            } else {
                tree.add_file(&segments);
            }
        }
        tree
    }

    fn resolve_in(files: &[&str], name: &str) -> Result<Resolved> {
        resolve(&tree(files), &name.parse().unwrap(), None)
    }

    fn resolved_path(files: &[&str], name: &str) -> String {
        let resolved = resolve_in(files, name).unwrap();
        assert_eq!(resolved.warning, None);
        resolved.path.to_string()
    }

    fn error(files: &[&str], name: &str) -> String {
        format!("{:#}", resolve_in(files, name).unwrap_err())
    }

    #[test]
    fn explicit_path_wins_and_must_exist() {
        let files = ["addons/x/plugin.cfg", "other/y.gd"];
        let path: ArchivePath = "other".parse().unwrap();
        let resolved = resolve(&tree(&files), &"x".parse().unwrap(), Some(&path)).unwrap();
        assert_eq!(resolved.path, path);

        let missing: ArchivePath = "nope".parse().unwrap();
        let err = resolve(&tree(&files), &"x".parse().unwrap(), Some(&missing)).unwrap_err();
        assert!(
            err.to_string()
                .contains("`path = \"nope\"` is not a folder")
        );
    }

    #[test]
    fn standard_addons_layout() {
        let files = [
            "README.md",
            "LICENSE",
            "addons/slang/plugin.cfg",
            "addons/other/a.gd",
        ];
        assert_eq!(resolved_path(&files, "slang"), "addons/slang");
    }

    #[test]
    fn addons_layout_inside_wrapper_folder() {
        let files = [
            "slang-v1.2/README.md",
            "slang-v1.2/addons/slang/slang.gdextension",
        ];
        assert_eq!(resolved_path(&files, "slang"), "slang-v1.2/addons/slang");
    }

    #[test]
    fn macos_metadata_does_not_hide_the_wrapper() {
        let files = ["__MACOSX/._x", "w/addons/slang/plugin.cfg"];
        assert_eq!(resolved_path(&files, "slang"), "w/addons/slang");
    }

    #[test]
    fn root_that_is_an_addon_folder() {
        let files = ["plugin.cfg", "plugin.gd"];
        assert_eq!(resolved_path(&files, "slang"), ".");
        let files = ["slang.gdextension", "bin/libslang.so"];
        assert_eq!(resolved_path(&files, "slang"), ".");
    }

    #[test]
    fn single_top_level_addon_folder() {
        let files = ["slang/slang.gdextension", "slang/bin/x.dll"];
        assert_eq!(resolved_path(&files, "anything"), "slang");
    }

    #[test]
    fn lone_differently_named_addon_warns() {
        let resolved = resolve_in(&["addons/slang_gd/plugin.cfg"], "godot-slang").unwrap();
        assert_eq!(resolved.path.to_string(), "addons/slang_gd");
        let warning = resolved.warning.unwrap();
        assert!(warning.contains("res:// paths"), "{warning}");
    }

    #[test]
    fn multiple_addons_are_ambiguous() {
        let err = error(&["addons/a/plugin.cfg", "addons/b/plugin.cfg"], "c");
        assert!(err.contains("cannot tell which folder"), "{err}");
        assert!(err.contains("addons/a/, addons/b/"), "{err}");
        assert!(err.contains("Set `path`"), "{err}");
    }

    #[test]
    fn unrecognized_layout_lists_top_level() {
        let err = error(&["src/a.gd", "docs/", "README.md"], "c");
        assert!(err.contains("cannot find the addon folder"), "{err}");
        assert!(err.contains("README.md, docs/, src/"), "{err}");
    }

    #[test]
    fn directory_tree_matches_by_folder_name() {
        let temp = tempfile::tempdir().unwrap();
        let addon = temp.path().join("godot-slang");
        std::fs::create_dir_all(addon.join("bin")).unwrap();
        std::fs::write(addon.join("bin").join("x.dll"), "").unwrap();

        let resolved = resolve(&DirTree::new(&addon), &"godot-slang".parse().unwrap(), None);
        assert_eq!(resolved.unwrap().path, ArchivePath::root());
        assert!(resolve(&DirTree::new(&addon), &"other".parse().unwrap(), None).is_err());
    }

    #[test]
    fn directory_tree_finds_addons_layout_in_build_output() {
        let temp = tempfile::tempdir().unwrap();
        let addon = temp.path().join("addons").join("godot-slang");
        std::fs::create_dir_all(&addon).unwrap();
        std::fs::write(addon.join("slang.gdextension"), "").unwrap();

        let resolved = resolve(
            &DirTree::new(temp.path()),
            &"godot-slang".parse().unwrap(),
            None,
        );
        assert_eq!(resolved.unwrap().path.to_string(), "addons/godot-slang");
    }
}
