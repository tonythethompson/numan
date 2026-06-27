use anyhow::{Context, Result};
use serde::Serialize;
use std::io::Write;
use std::path::Path;

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
