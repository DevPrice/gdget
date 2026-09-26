use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::manifest::{MANIFEST_FILE, OVERRIDES_FILE};

const GODOT_PROJECT_FILE: &str = "project.godot";

/// A Godot project root: the directory holding `addons.toml` and/or `project.godot`.
#[derive(Debug, Clone)]
pub struct Project {
    root: PathBuf,
}

impl Project {
    /// Finds the nearest ancestor of `start` (inclusive) containing `addons.toml` or
    /// `project.godot`. Stopping at the first of either keeps a game nested in a larger
    /// repo from picking up an unrelated manifest further up.
    pub fn discover(start: &Path) -> Result<Self> {
        for dir in start.ancestors() {
            if dir.join(MANIFEST_FILE).is_file() || dir.join(GODOT_PROJECT_FILE).is_file() {
                return Ok(Self {
                    root: dir.to_owned(),
                });
            }
        }
        bail!(
            "no {MANIFEST_FILE} or {GODOT_PROJECT_FILE} found in {} or any parent directory",
            start.display()
        )
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.root.join(MANIFEST_FILE)
    }

    pub fn overrides_path(&self) -> PathBuf {
        self.root.join(OVERRIDES_FILE)
    }

    pub fn addons_dir(&self) -> PathBuf {
        self.root.join("addons")
    }

    /// gdget's working directory for staging, trash and link records. It must be inside
    /// the project so renames into `addons/` stay on one volume and are atomic.
    pub fn state_dir(&self) -> PathBuf {
        self.root.join(".gdget")
    }

    pub fn has_manifest(&self) -> bool {
        self.manifest_path().is_file()
    }

    pub fn has_godot_project(&self) -> bool {
        self.root.join(GODOT_PROJECT_FILE).is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_nearest_marker_file() {
        let temp = tempfile::tempdir().unwrap();
        let outer = temp.path();
        let game = outer.join("games").join("mygame");
        let deep = game.join("scenes").join("level1");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(outer.join(MANIFEST_FILE), "").unwrap();
        std::fs::write(game.join(GODOT_PROJECT_FILE), "").unwrap();

        let project = Project::discover(&deep).unwrap();
        assert_eq!(project.root(), game);
        assert!(project.has_godot_project());
        assert!(!project.has_manifest());

        assert_eq!(
            Project::discover(&outer.join("games")).unwrap().root(),
            outer
        );
    }

    #[test]
    fn errors_without_any_marker_file() {
        let temp = tempfile::tempdir().unwrap();
        let err = Project::discover(temp.path()).unwrap_err().to_string();
        assert!(
            err.contains("no addons.toml or project.godot found"),
            "{err}"
        );
    }
}
