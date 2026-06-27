use anyhow::{Context, Result};
use serde::Serialize;
use std::io::Write;
use std::path::Path;

/// Write arbitrary bytes to `path` atomically.
///
/// Writes to a temp file in the same directory as `path`, flushes, then
/// renames (on Windows: `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`).
/// The destination is fully replaced or left entirely unchanged — there is
/// no window in which a partial write is visible.
///
/// `content` must already be UTF-8 or arbitrary binary data as needed by the
/// caller.  For JSON payloads, prefer [`write_json_atomic`].
pub fn write_bytes_atomic(path: &Path, content: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create directory '{}'", parent.display()))?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("Failed to create temp file in '{}'", parent.display()))?;
    tmp.write_all(content)
        .context("Failed to write temp file")?;
    tmp.flush().context("Failed to flush temp file")?;
    tmp.persist(path).map_err(|e| {
        anyhow::anyhow!(
            "Failed to atomically write '{}': {}",
            path.display(),
            e.error
        )
    })?;
    Ok(())
}

/// Serialize `value` to JSON and write it to `path` atomically.
///
/// Writes to a temp file in the same directory as `path`, then renames.
/// On Windows, `NamedTempFile::persist` uses `MoveFileExW` with
/// `MOVEFILE_REPLACE_EXISTING`, so the destination is replaced atomically.
pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let content = serde_json::to_string_pretty(value).context("Failed to serialize JSON")?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create directory '{}'", parent.display()))?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("Failed to create temp file in '{}'", parent.display()))?;
    tmp.write_all(content.as_bytes())
        .context("Failed to write temp file")?;
    tmp.flush().context("Failed to flush temp file")?;
    tmp.persist(path).map_err(|e| {
        anyhow::anyhow!(
            "Failed to atomically write '{}': {}",
            path.display(),
            e.error
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Dummy {
        val: u32,
    }

    // ── write_bytes_atomic ───────────────────────────────────────────────────

    #[test]
    fn bytes_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.nu");
        let content = b"# Generated and managed by Numan. Do not edit.\nuse \"foo.nu\"\n";
        write_bytes_atomic(&path, content).unwrap();
        let read_back = std::fs::read(&path).unwrap();
        assert_eq!(read_back, content);
    }

    #[test]
    fn bytes_overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.nu");
        write_bytes_atomic(&path, b"first").unwrap();
        write_bytes_atomic(&path, b"second").unwrap();
        let read_back = std::fs::read(&path).unwrap();
        assert_eq!(read_back, b"second");
    }

    #[test]
    fn bytes_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("deep").join("file.nu");
        write_bytes_atomic(&path, b"content").unwrap();
        assert!(path.exists());
    }

    // ── write_json_atomic ────────────────────────────────────────────────────

    #[test]
    fn roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.json");
        write_json_atomic(&path, &Dummy { val: 42 }).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: Dummy = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed, Dummy { val: 42 });
    }

    #[test]
    fn overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.json");
        write_json_atomic(&path, &Dummy { val: 1 }).unwrap();
        write_json_atomic(&path, &Dummy { val: 2 }).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: Dummy = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed.val, 2);
    }
}
