use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use tempfile::TempDir;

const TRASH_PREFIX: &str = "old-";

/// gdget's scratch space under `.gdget/`, next to `addons/` so every rename between them
/// stays on one volume and is atomic.
pub struct Workspace {
    staging: PathBuf,
    trash: PathBuf,
}

/// A folder being prepared for install. Dropping it without [`Workspace::swap_in`]
/// deletes it, so failed installs leave nothing behind.
pub struct Staged {
    shell: TempDir,
}

impl Staged {
    /// Where the new addon contents go. Does not exist until the caller creates it.
    pub fn path(&self) -> PathBuf {
        self.shell.path().join("addon")
    }
}

impl Workspace {
    /// Creates `.gdget/` if needed and clears what an interrupted run left in it. Rescued
    /// folders are kept for the user to recover.
    pub fn prepare(state_dir: &Path) -> Result<Self> {
        let workspace = Self {
            staging: state_dir.join("staging"),
            trash: state_dir.join("trash"),
        };
        let _ = std::fs::remove_dir_all(&workspace.staging);
        if let Ok(entries) = std::fs::read_dir(&workspace.trash) {
            for entry in entries.flatten() {
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(TRASH_PREFIX)
                {
                    let _ = std::fs::remove_dir_all(entry.path());
                }
            }
        }
        for dir in [&workspace.staging, &workspace.trash] {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("cannot create {}", dir.display()))?;
        }
        // Hidden folders are already skipped by Godot's importer; this makes it explicit
        // in case the editor has the project open while a sync stages files.
        let gdignore = state_dir.join(".gdignore");
        if !gdignore.exists() {
            std::fs::write(&gdignore, "")
                .with_context(|| format!("cannot write {}", gdignore.display()))?;
        }
        Ok(workspace)
    }

    pub fn stage(&self, name: &str) -> Result<Staged> {
        let shell = tempfile::Builder::new()
            .prefix(&format!("{name}-"))
            .tempdir_in(&self.staging)
            .with_context(|| format!("cannot create a folder in {}", self.staging.display()))?;
        Ok(Staged { shell })
    }

    /// Replaces `target` with the staged folder. Either the new folder is in place, or
    /// `target` is unchanged and an error explains why.
    pub fn swap_in(&self, staged: Staged, target: &Path) -> Result<()> {
        let old = self.move_to_trash(target)?;
        if let Err(e) = std::fs::rename(staged.path(), target) {
            if let Some(old) = old
                && let Err(restore) = std::fs::rename(old.path().join("addon"), target)
            {
                let rescued = self.rescue(old);
                return Err(restore).with_context(|| {
                    format!(
                        "cannot install {} ({e}) and cannot put the previous version back; \
                         it is in {}",
                        target.display(),
                        rescued.display()
                    )
                });
            }
            return Err(e).with_context(|| format!("cannot install {}", target.display()));
        }
        Ok(())
    }

    /// Keeps a trashed folder out of the next run's cleanup and returns where it is.
    fn rescue(&self, slot: TempDir) -> PathBuf {
        let kept = slot.keep();
        let name = kept.file_name().unwrap_or_default().to_string_lossy();
        let rescued = self.trash.join(name.replacen(TRASH_PREFIX, "rescued-", 1));
        match std::fs::rename(&kept, &rescued) {
            Ok(()) => rescued.join("addon"),
            Err(_) => kept.join("addon"),
        }
    }

    /// Removes `target`, or leaves it untouched and explains why not.
    pub fn remove(&self, target: &Path) -> Result<()> {
        self.move_to_trash(target).map(drop)
    }

    /// Moves `target` into the trash, which is deleted when the returned guard drops.
    /// Deleting after the move means a locked file fails the rename, before anything
    /// has changed, rather than halfway through a recursive delete.
    fn move_to_trash(&self, target: &Path) -> Result<Option<TempDir>> {
        if target.symlink_metadata().is_err() {
            return Ok(None);
        }
        if let Some(locked) = find_locked_file(target) {
            return Err(in_use(&locked));
        }
        let slot = tempfile::Builder::new()
            .prefix(TRASH_PREFIX)
            .tempdir_in(&self.trash)
            .with_context(|| format!("cannot create a folder in {}", self.trash.display()))?;
        std::fs::rename(target, slot.path().join("addon"))
            .map_err(|e| {
                if is_sharing_error(&e) {
                    in_use(target)
                } else {
                    e.into()
                }
            })
            .with_context(|| format!("cannot replace {}", target.display()))?;
        Ok(Some(slot))
    }
}

fn in_use(path: &Path) -> anyhow::Error {
    anyhow!(
        "{} is in use, most likely by the Godot editor with this addon's library loaded. \
         Close the editor and run gdget again",
        path.display()
    )
}

fn is_sharing_error(error: &io::Error) -> bool {
    const ERROR_ACCESS_DENIED: i32 = 5;
    const ERROR_SHARING_VIOLATION: i32 = 32;
    cfg!(windows)
        && matches!(
            error.raw_os_error(),
            Some(ERROR_ACCESS_DENIED | ERROR_SHARING_VIOLATION)
        )
}

/// Windows lets a folder be renamed while a DLL inside it is loaded, so without this
/// check the swap would succeed and leave the editor running the old library out of the
/// trash. A loaded image refuses write access, which can be probed without changing it.
#[cfg(windows)]
fn find_locked_file(dir: &Path) -> Option<PathBuf> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ_WRITE_DELETE: u32 = 0x1 | 0x2 | 0x4;
    const ERROR_SHARING_VIOLATION: i32 = 32;

    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_dir() {
            if let Some(locked) = find_locked_file(&path) {
                return Some(locked);
            }
        } else if file_type.is_file() {
            let probe = std::fs::OpenOptions::new()
                .write(true)
                .share_mode(FILE_SHARE_READ_WRITE_DELETE)
                .open(&path);
            if probe.is_err_and(|e| e.raw_os_error() == Some(ERROR_SHARING_VIOLATION)) {
                return Some(path);
            }
        }
    }
    None
}

#[cfg(not(windows))]
fn find_locked_file(_dir: &Path) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(dir: &Path, contents: &str) {
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(dir.join("bin").join("lib.dll"), contents).unwrap();
    }

    fn read(dir: &Path) -> String {
        std::fs::read_to_string(dir.join("bin").join("lib.dll")).unwrap()
    }

    fn is_empty(dir: &Path) -> bool {
        std::fs::read_dir(dir).unwrap().next().is_none()
    }

    #[test]
    fn prepare_clears_leftovers_and_hides_itself_from_godot() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join(".gdget");
        fill(&state.join("staging").join("stale"), "x");
        fill(&state.join("trash").join("old-1"), "x");
        fill(&state.join("trash").join("rescued-2"), "x");
        Workspace::prepare(&state).unwrap();
        assert!(is_empty(&state.join("staging")));
        assert!(!state.join("trash").join("old-1").exists());
        assert!(state.join("trash").join("rescued-2").exists());
        assert!(state.join(".gdignore").is_file());
    }

    #[test]
    fn installs_fresh_and_replaces_existing() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join(".gdget");
        let target = temp.path().join("addons").join("a");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        let workspace = Workspace::prepare(&state).unwrap();

        let staged = workspace.stage("a").unwrap();
        fill(&staged.path(), "v1");
        workspace.swap_in(staged, &target).unwrap();
        assert_eq!(read(&target), "v1");

        let staged = workspace.stage("a").unwrap();
        fill(&staged.path(), "v2");
        workspace.swap_in(staged, &target).unwrap();
        assert_eq!(read(&target), "v2");

        assert!(is_empty(&state.join("staging")));
        assert!(is_empty(&state.join("trash")));
    }

    #[test]
    fn dropped_stage_leaves_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join(".gdget");
        let workspace = Workspace::prepare(&state).unwrap();
        let staged = workspace.stage("a").unwrap();
        fill(&staged.path(), "half");
        drop(staged);
        assert!(is_empty(&state.join("staging")));
    }

    #[test]
    fn removes_and_tolerates_missing() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = Workspace::prepare(&temp.path().join(".gdget")).unwrap();
        let target = temp.path().join("a");
        fill(&target, "x");
        workspace.remove(&target).unwrap();
        assert!(!target.exists());
        workspace.remove(&target).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn loaded_library_blocks_replacement_until_unloaded() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join(".gdget");
        let target = temp.path().join("a");
        std::fs::create_dir_all(target.join("bin")).unwrap();
        let system = std::env::var_os("SystemRoot").unwrap();
        let library = target.join("bin").join("lib.dll");
        std::fs::copy(
            Path::new(&system).join("System32").join("version.dll"),
            &library,
        )
        .unwrap();
        let workspace = Workspace::prepare(&state).unwrap();

        // SAFETY: version.dll is a system library with no initialization side effects.
        let loaded = unsafe { libloading::Library::new(&library) }.unwrap();
        let staged = workspace.stage("a").unwrap();
        fill(&staged.path(), "new");
        let err = format!("{:#}", workspace.swap_in(staged, &target).unwrap_err());
        assert!(err.contains("lib.dll is in use"), "{err}");
        assert!(library.is_file());
        assert!(is_empty(&state.join("staging")));
        assert!(is_empty(&state.join("trash")));

        drop(loaded);
        let staged = workspace.stage("a").unwrap();
        fill(&staged.path(), "new");
        workspace.swap_in(staged, &target).unwrap();
        assert_eq!(read(&target), "new");
    }

    #[cfg(windows)]
    #[test]
    fn locked_file_fails_cleanly_and_keeps_the_old_install() {
        use std::os::windows::fs::OpenOptionsExt;

        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join(".gdget");
        let target = temp.path().join("a");
        fill(&target, "loaded");
        let workspace = Workspace::prepare(&state).unwrap();
        let _lock = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(target.join("bin").join("lib.dll"))
            .unwrap();

        let staged = workspace.stage("a").unwrap();
        fill(&staged.path(), "new");
        let err = format!("{:#}", workspace.swap_in(staged, &target).unwrap_err());
        assert!(err.contains("Close the editor"), "{err}");
        drop(_lock);

        assert_eq!(read(&target), "loaded");
        assert!(is_empty(&state.join("staging")));
        assert!(is_empty(&state.join("trash")));

        let err = format!("{:#}", {
            let _lock = std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(target.join("bin").join("lib.dll"))
                .unwrap();
            workspace.remove(&target).unwrap_err()
        });
        assert!(err.contains("Close the editor"), "{err}");
        assert!(target.exists());
    }
}
