use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use tempfile::TempDir;

use crate::link;

const TRASH_PREFIX: &str = "old-";

/// gdget's scratch space under `.gdget/`, next to `addons/` so every rename between them
/// stays on one volume and is atomic. Holds an exclusive lock on the project while alive.
pub struct Workspace {
    staging: PathBuf,
    trash: PathBuf,
    _lock: File,
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
    /// Creates `.gdget/` if needed, locks the project against other gdget runs, and
    /// clears what an interrupted run left behind. Rescued folders are kept for the user
    /// to recover.
    pub fn prepare(state_dir: &Path) -> Result<Self> {
        ensure_real_dir(state_dir)?;
        let lock = lock_project(state_dir)?;
        let workspace = Self {
            staging: state_dir.join("staging"),
            trash: state_dir.join("trash"),
            _lock: lock,
        };
        for dir in [&workspace.staging, &workspace.trash] {
            ensure_real_dir(dir)?;
        }
        remove_entries(&workspace.staging, |_| true);
        remove_entries(&workspace.trash, |name| name.starts_with(TRASH_PREFIX));
        // Hidden folders are already skipped by Godot's importer; this makes it explicit
        // in case the editor has the project open while a sync stages files. create_new
        // refuses to write through a planted symlink.
        let gdignore = state_dir.join(".gdignore");
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&gdignore)
        {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => {
                return Err(e).with_context(|| format!("cannot write {}", gdignore.display()));
            }
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
        self.replace(target, || {
            std::fs::rename(staged.path(), target)
                .with_context(|| format!("cannot install {}", target.display()))
        })
    }

    /// Replaces `target` with a link to `source`, with the same guarantee as `swap_in`.
    pub fn link_in(&self, source: &Path, target: &Path) -> Result<()> {
        self.replace(target, || link::create(source, target))
    }

    /// Removes `target` (for a link, only the link), or leaves it untouched and explains
    /// why not.
    pub fn remove(&self, target: &Path) -> Result<()> {
        self.take_old(target).map(drop)
    }

    fn replace(&self, target: &Path, put: impl FnOnce() -> Result<()>) -> Result<()> {
        let old = self.take_old(target)?;
        let Err(error) = put() else {
            return Ok(());
        };
        let restored = match old {
            None => Ok(()),
            Some(Old::Link(source)) => link::create(&source, target),
            Some(Old::Dir(slot)) => {
                std::fs::rename(slot.path().join("addon"), target).map_err(|e| {
                    let rescued = self.rescue(slot);
                    anyhow!("{e}; the previous version is in {}", rescued.display())
                })
            }
        };
        match restored {
            Ok(()) => Err(error),
            Err(restore) => Err(error.context(format!(
                "cannot put the previous {} back: {restore:#}",
                target.display()
            ))),
        }
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

    /// Clears `target` out of the way, returning what is needed to put it back.
    ///
    /// Folders move into the trash, which is deleted when the returned guard drops, so a
    /// locked file fails the rename before anything has changed rather than halfway
    /// through a recursive delete. Links are removed directly and never moved into the
    /// trash, where a recursive delete could reach through them into the linked folder.
    fn take_old(&self, target: &Path) -> Result<Option<Old>> {
        let meta = match target.symlink_metadata() {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("cannot read {}", target.display())),
        };
        if let Some(locked) = find_locked_file(target) {
            return Err(in_use(&locked));
        }
        if meta.file_type().is_symlink() {
            let source = link::target(target)?;
            link::remove(target)?;
            return Ok(Some(Old::Link(source)));
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
        Ok(Some(Old::Dir(slot)))
    }
}

/// Creates `dir` if it is missing and refuses anything but a real directory there. A
/// symlink or junction committed in place of `addons/` or `.gdget/` would otherwise
/// redirect gdget's writes and deletes outside the project.
pub fn ensure_real_dir(dir: &Path) -> Result<()> {
    match dir.symlink_metadata() {
        Ok(meta) if meta.file_type().is_symlink() => bail!(
            "{} is a link; gdget only writes to a real folder there. Replace the link with \
             a folder",
            dir.display()
        ),
        Ok(meta) if meta.is_dir() => Ok(()),
        Ok(_) => bail!("{} is not a folder", dir.display()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            std::fs::create_dir(dir).with_context(|| format!("cannot create {}", dir.display()))
        }
        Err(e) => Err(e).with_context(|| format!("cannot read {}", dir.display())),
    }
}

/// Best effort: anything that can't be deleted now is retried on the next run.
fn remove_entries(dir: &Path, selected: impl Fn(&str) -> bool) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if selected(&entry.file_name().to_string_lossy()) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

fn lock_project(state_dir: &Path) -> Result<File> {
    let path = state_dir.join("lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .with_context(|| format!("cannot open {}", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => {
            bail!("another gdget is already running in this project; wait for it to finish")
        }
        Err(TryLockError::Error(e)) => {
            Err(e).with_context(|| format!("cannot lock {}", path.display()))
        }
    }
}

/// What `take_old` moved out of the way.
enum Old {
    Dir(TempDir),
    Link(PathBuf),
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
    fn refuses_a_linked_state_dir_and_leaves_the_target_alone() {
        let temp = tempfile::tempdir().unwrap();
        let elsewhere = temp.path().join("elsewhere");
        fill(&elsewhere.join("staging").join("precious"), "keep");
        let state = temp.path().join(".gdget");
        link::create(&elsewhere, &state).unwrap();

        let err = Workspace::prepare(&state).err().unwrap().to_string();
        assert!(err.contains("is a link"), "{err}");
        assert_eq!(read(&elsewhere.join("staging").join("precious")), "keep");
        link::remove(&state).unwrap();
    }

    #[test]
    fn only_one_workspace_per_project_at_a_time() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join(".gdget");
        let first = Workspace::prepare(&state).unwrap();
        let err = Workspace::prepare(&state).err().unwrap().to_string();
        assert!(err.contains("another gdget is already running"), "{err}");
        drop(first);
        Workspace::prepare(&state).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn does_not_write_gdignore_through_a_planted_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join(".gdget");
        std::fs::create_dir(&state).unwrap();
        let victim = temp.path().join("victim");
        std::os::unix::fs::symlink(&victim, state.join(".gdignore")).unwrap();
        Workspace::prepare(&state).unwrap();
        assert!(!victim.exists());
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
    fn links_replace_copies_and_back_without_touching_the_source() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = Workspace::prepare(&temp.path().join(".gdget")).unwrap();
        let source = temp.path().join("dev");
        fill(&source, "dev build");
        let target = temp.path().join("a");

        let staged = workspace.stage("a").unwrap();
        fill(&staged.path(), "pinned");
        workspace.swap_in(staged, &target).unwrap();

        workspace.link_in(&source, &target).unwrap();
        assert!(link::is_link(&target));
        assert_eq!(read(&target), "dev build");

        let staged = workspace.stage("a").unwrap();
        fill(&staged.path(), "pinned");
        workspace.swap_in(staged, &target).unwrap();
        assert!(!link::is_link(&target));
        assert_eq!(read(&target), "pinned");
        assert_eq!(read(&source), "dev build");

        workspace.link_in(&source, &target).unwrap();
        workspace.remove(&target).unwrap();
        assert!(target.symlink_metadata().is_err());
        assert_eq!(read(&source), "dev build");
        assert!(is_empty(&temp.path().join(".gdget").join("trash")));
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
