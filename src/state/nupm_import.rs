use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use crate::util::atomic::write_json_atomic;

const IMPORTS_FILE: &str = "state/nupm-imports.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NupmImportsFile {
    pub version: u32,
    pub imports: BTreeMap<String, NupmImportRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NupmImportRecord {
    pub trust_level: String,
    pub nupm_source_path: String,
    pub nupm_metadata_path: String,
    pub nupm_metadata_sha256: String,
    pub source_payload_sha256: String,
    pub imported_payload_sha256: String,
    #[serde(default)]
    pub observed_git_remote: Option<String>,
    #[serde(default)]
    pub observed_git_commit: Option<String>,
    pub imported_at: String,
}

impl NupmImportsFile {
    pub fn empty() -> Self {
        Self {
            version: 1,
            imports: BTreeMap::new(),
        }
    }

    fn path(root: &Path) -> std::path::PathBuf {
        root.join(IMPORTS_FILE)
    }

    pub fn load(root: &Path) -> Result<Self> {
        let path = Self::path(root);
        if !path.exists() {
            return Ok(Self::empty());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        Ok(serde_json::from_str(&content)?)
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        write_json_atomic(&Self::path(root), self)
    }

    pub fn upsert(&mut self, package_id: &str, record: NupmImportRecord) {
        self.imports.insert(package_id.to_string(), record);
    }

    pub fn remove(&mut self, package_id: &str) -> bool {
        self.imports.remove(package_id).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_and_remove_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut file = NupmImportsFile::empty();
        file.upsert(
            "owner/pkg",
            NupmImportRecord {
                trust_level: "local_foreign_import".to_string(),
                nupm_source_path: "/src".to_string(),
                nupm_metadata_path: "/src/nupm.nuon".to_string(),
                nupm_metadata_sha256: "meta".to_string(),
                source_payload_sha256: "src".to_string(),
                imported_payload_sha256: "imp".to_string(),
                observed_git_remote: None,
                observed_git_commit: None,
                imported_at: "2026-01-01T00:00:00Z".to_string(),
            },
        );
        file.save(root).unwrap();
        let loaded = NupmImportsFile::load(root).unwrap();
        assert_eq!(loaded.imports.len(), 1);
        let mut again = loaded;
        assert!(again.remove("owner/pkg"));
        again.save(root).unwrap();
        assert!(NupmImportsFile::load(root).unwrap().imports.is_empty());
    }
}
