use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use crate::util::atomic::write_json_atomic;

const IMPORTS_FILE: &str = "state/nupm-imports.json";
const IMPORTS_FILE_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NupmImportsFile {
    pub version: u32,
    pub imports: BTreeMap<String, NupmImportRecord>,
}

/// Typed reason explaining why a specific nupm source was selected for import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NupmSelectionReason {
    /// Imported because the package is a module with a mod.nu entry point.
    ModuleEntry,
}

/// Typed transformation performed during import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NupmTransformation {
    /// Copied the regular module tree as-is (no build, no script execution).
    CopiedModuleTree,
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
    /// Original nupm package name declared in nupm.nuon.
    #[serde(default = "default_unknown")]
    pub original_nupm_name: String,
    /// Original nupm package version declared in nupm.nuon.
    #[serde(default = "default_unknown")]
    pub original_nupm_version: String,
    /// Why this specific source shape was selected for import.
    #[serde(default = "default_selection_reason")]
    pub selection_reason: NupmSelectionReason,
    /// Transformation applied to the source during import.
    #[serde(default = "default_transformation")]
    pub transformation_performed: NupmTransformation,
}

fn default_unknown() -> String {
    "unknown".to_string()
}

fn default_selection_reason() -> NupmSelectionReason {
    NupmSelectionReason::ModuleEntry
}

fn default_transformation() -> NupmTransformation {
    NupmTransformation::CopiedModuleTree
}

impl NupmImportsFile {
    pub fn empty() -> Self {
        Self {
            version: IMPORTS_FILE_VERSION,
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
        let mut loaded: NupmImportsFile = serde_json::from_str(&content)?;
        if loaded.version < IMPORTS_FILE_VERSION {
            loaded = migrate_v1_to_v2(loaded);
        }
        Ok(loaded)
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

fn migrate_v1_to_v2(old: NupmImportsFile) -> NupmImportsFile {
    let imports = old
        .imports
        .into_iter()
        .map(|(package_id, mut record)| {
            if record.original_nupm_name.is_empty() {
                record.original_nupm_name = "unknown".to_string();
            }
            if record.original_nupm_version.is_empty() {
                record.original_nupm_version = "unknown".to_string();
            }
            (package_id, record)
        })
        .collect();
    NupmImportsFile {
        version: IMPORTS_FILE_VERSION,
        imports,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_record() -> NupmImportRecord {
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
            original_nupm_name: "minimal".to_string(),
            original_nupm_version: "0.1.0".to_string(),
            selection_reason: NupmSelectionReason::ModuleEntry,
            transformation_performed: NupmTransformation::CopiedModuleTree,
        }
    }

    #[test]
    fn upsert_and_remove_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut file = NupmImportsFile::empty();
        file.upsert("owner/pkg", sample_record());
        file.save(root).unwrap();
        let loaded = NupmImportsFile::load(root).unwrap();
        assert_eq!(loaded.imports.len(), 1);
        assert_eq!(loaded.version, 2);
        let record = loaded.imports.get("owner/pkg").unwrap();
        assert_eq!(record.original_nupm_name, "minimal");
        assert_eq!(record.original_nupm_version, "0.1.0");
        assert_eq!(record.selection_reason, NupmSelectionReason::ModuleEntry);
        assert_eq!(
            record.transformation_performed,
            NupmTransformation::CopiedModuleTree
        );
        let mut again = loaded;
        assert!(again.remove("owner/pkg"));
        again.save(root).unwrap();
        assert!(NupmImportsFile::load(root).unwrap().imports.is_empty());
    }

    #[test]
    fn v1_record_loads_with_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let v1 = r#"{"version":1,"imports":{"owner/pkg":{"trust_level":"local_foreign_import","nupm_source_path":"/src","nupm_metadata_path":"/src/nupm.nuon","nupm_metadata_sha256":"meta","source_payload_sha256":"src","imported_payload_sha256":"imp","imported_at":"2026-01-01T00:00:00Z"}}}"#;
        std::fs::create_dir_all(root.join("state")).unwrap();
        std::fs::write(root.join(IMPORTS_FILE), v1).unwrap();
        let loaded = NupmImportsFile::load(root).unwrap();
        assert_eq!(loaded.version, 2);
        let record = loaded.imports.get("owner/pkg").unwrap();
        assert_eq!(record.original_nupm_name, "unknown");
        assert_eq!(record.original_nupm_version, "unknown");
        assert_eq!(record.selection_reason, NupmSelectionReason::ModuleEntry);
        assert_eq!(
            record.transformation_performed,
            NupmTransformation::CopiedModuleTree
        );
    }
}
