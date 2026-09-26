use std::io::Write;
use std::path::Path;

/// Replaces `path` with `contents` so readers see either the old or the new file, never a
/// partial write.
pub fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    temp.write_all(contents)?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    Ok(())
}
