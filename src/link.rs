use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Links `link` to the directory `target`: a junction on Windows, which needs neither
/// admin rights nor Developer Mode, and a symlink elsewhere.
pub fn create(target: &Path, link: &Path) -> Result<()> {
    let target = std::path::absolute(target)
        .with_context(|| format!("cannot resolve {}", target.display()))?;
    #[cfg(windows)]
    let created = junction::create(&target, link);
    #[cfg(unix)]
    let created = std::os::unix::fs::symlink(&target, link);
    created.with_context(|| format!("cannot link {} to {}", link.display(), target.display()))
}

/// Whether `path` is a symlink or junction (std reports both as symlinks).
pub fn is_link(path: &Path) -> bool {
    path.symlink_metadata()
        .is_ok_and(|meta| meta.file_type().is_symlink())
}

pub fn target(link: &Path) -> Result<PathBuf> {
    #[cfg(windows)]
    let target = if junction::exists(link).unwrap_or(false) {
        junction::get_target(link)
    } else {
        std::fs::read_link(link)
    };
    #[cfg(unix)]
    let target = std::fs::read_link(link);
    target.with_context(|| format!("cannot read link {}", link.display()))
}

/// Removes the link itself. Never touches what it points to.
pub fn remove(link: &Path) -> Result<()> {
    // Directory junctions and directory symlinks are removed with RemoveDirectory on
    // Windows; a symlink on Unix is a file.
    #[cfg(windows)]
    let removed = std::fs::remove_dir(link).or_else(|_| std::fs::remove_file(link));
    #[cfg(unix)]
    let removed = std::fs::remove_file(link);
    removed.with_context(|| format!("cannot remove link {}", link.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_round_trip_leaves_target_intact() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("dev").join("addon");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("plugin.cfg"), "x").unwrap();
        let link = temp.path().join("link");

        create(&target, &link).unwrap();
        assert!(is_link(&link));
        assert!(!is_link(&target));
        assert_eq!(
            std::fs::read_to_string(link.join("plugin.cfg")).unwrap(),
            "x"
        );
        assert_eq!(
            std::fs::canonicalize(super::target(&link).unwrap()).unwrap(),
            std::fs::canonicalize(&target).unwrap()
        );

        remove(&link).unwrap();
        assert!(link.symlink_metadata().is_err());
        assert_eq!(
            std::fs::read_to_string(target.join("plugin.cfg")).unwrap(),
            "x"
        );
    }

    #[test]
    fn relative_targets_are_made_absolute() {
        let temp = tempfile::tempdir().unwrap();
        let link = temp.path().join("link");
        create(Path::new("."), &link).unwrap();
        assert!(super::target(&link).unwrap().is_absolute());
        remove(&link).unwrap();
    }
}
