use std::io::Write;
use std::path::{Component, Path, PathBuf};

/// Replaces `path` with `contents` so readers see either the old or the new file, never a
/// partial write.
pub(crate) fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    temp.write_all(contents)?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    Ok(())
}

/// `path` relative to `base`, both absolute, with `.` and `..` resolved lexically. `None`
/// when they share no root, such as different drives on Windows.
pub(crate) fn relative_path(path: &Path, base: &Path) -> Option<PathBuf> {
    let path = normalize(path);
    let base = normalize(base);
    if path.first() != base.first() {
        return None;
    }
    let common = path.iter().zip(&base).take_while(|(a, b)| a == b).count();
    let ups = std::iter::repeat_n(Component::ParentDir, base.len() - common);
    Some(ups.chain(path[common..].iter().copied()).collect())
}

fn normalize(path: &Path) -> Vec<Component<'_>> {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(components.last(), Some(Component::Normal(_))) {
                    components.pop();
                }
            }
            other => components.push(other),
        }
    }
    components
}

#[cfg(test)]
mod tests {
    use super::*;

    fn abs(path: &str) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(format!("C:{}", path.replace('/', "\\")))
        } else {
            PathBuf::from(path)
        }
    }

    fn relative(path: &str, base: &str) -> String {
        let relative = relative_path(&abs(path), &abs(base)).unwrap();
        relative
            .components()
            .map(|c| c.as_os_str().to_str().unwrap())
            .collect::<Vec<_>>()
            .join("/")
    }

    #[test]
    fn relative_paths_walk_up_and_down() {
        assert_eq!(
            relative("/games/verse/addons/v", "/games/idle"),
            "../verse/addons/v"
        );
        assert_eq!(relative("/games/idle/dev/a", "/games/idle"), "dev/a");
        assert_eq!(
            relative("/games/idle/scenes/../dev/./a", "/games/idle"),
            "dev/a"
        );
        assert_eq!(relative("/games/idle", "/games/idle"), "");
    }

    #[cfg(windows)]
    #[test]
    fn different_drives_have_no_relative_path() {
        assert_eq!(
            relative_path(Path::new("D:\\builds\\a"), Path::new("C:\\games\\idle")),
            None
        );
    }
}
