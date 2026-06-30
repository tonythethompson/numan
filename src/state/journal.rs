use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::util::atomic::write_json_atomic;

/// Stage of an individual plugin registration attempt.
///
/// Lifecycle:
///   `prepared`   — journal written, `plugin add` not yet called
///   `registered` — `plugin add` exited 0, lockfile update pending
///   `failed`     — `plugin add` returned non-zero
///
/// After `registered`, the caller atomically persists the `PluginActivation`
/// record to the lockfile, then removes this entry from the journal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PendingStatus {
    Prepared,
    Registered,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingActivationEntry {
    pub package_id: String,
    pub payload_path: String,
    pub executable_path: String,
    /// Resolved absolute path to the plugin binary.
    pub absolute_binary_path: String,
    pub status: PendingStatus,
    #[serde(default)]
    pub error: Option<String>,
}

/// Pending-activation journal written to `<root>/state/pending-activation.json`.
///
/// Existence of this file after a command exits indicates an interrupted run.
/// `numan activate` reconciles it on next invocation (see `reconcile`).
/// `numan doctor` reports it without acting (see `docs/numan-doctor.md`).
///
/// A journal whose Nu identity (hash + version + registry path) does not match
/// the current `NuPaths` is stale and requires `numan init --refresh`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingActivation {
    pub nu_executable_sha256: String,
    pub nu_version: String,
    pub plugin_registry_path: String,
    pub created_at: String,
    pub entries: Vec<PendingActivationEntry>,
}

impl PendingActivation {
    fn path(root: &Path) -> PathBuf {
        root.join("state/pending-activation.json")
    }

    pub fn load(root: &Path) -> Result<Option<Self>> {
        let path = Self::path(root);
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read journal at '{}'", path.display()))?;
        let journal: Self =
            serde_json::from_str(&content).context("Failed to parse pending-activation journal")?;
        Ok(Some(journal))
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        write_json_atomic(&Self::path(root), self)
    }

    pub fn delete(root: &Path) -> Result<()> {
        let path = Self::path(root);
        if path.exists() {
            std::fs::remove_file(&path)
                .with_context(|| format!("Failed to remove journal at '{}'", path.display()))?;
        }
        Ok(())
    }

    /// Returns `true` when the journal's Nu identity matches the provided values.
    ///
    /// A mismatch means the journal is stale — the caller should refuse
    /// reuse and require `numan init --refresh`.
    pub fn matches_nu_identity(
        &self,
        nu_executable_sha256: &str,
        nu_version: &str,
        plugin_registry_path: &str,
    ) -> bool {
        self.nu_executable_sha256 == nu_executable_sha256
            && self.nu_version == nu_version
            && self.plugin_registry_path == plugin_registry_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_journal(root: &Path) -> PendingActivation {
        PendingActivation {
            nu_executable_sha256: "abc123".to_string(),
            nu_version: "0.113.1".to_string(),
            plugin_registry_path: root.join("plugins.msgpackz").to_string_lossy().to_string(),
            created_at: "0000000000000001".to_string(),
            entries: vec![PendingActivationEntry {
                package_id: "owner/plugin".to_string(),
                payload_path: "packages/plugins/owner/plugin/1.0.0-abc".to_string(),
                executable_path: "nu_plugin_thing".to_string(),
                absolute_binary_path:
                    "/numan/packages/plugins/owner/plugin/1.0.0-abc/nu_plugin_thing".to_string(),
                status: PendingStatus::Prepared,
                error: None,
            }],
        }
    }

    #[test]
    fn journal_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let journal = sample_journal(dir.path());
        journal.save(dir.path()).unwrap();
        let loaded = PendingActivation::load(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.nu_executable_sha256, "abc123");
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].status, PendingStatus::Prepared);
    }

    #[test]
    fn journal_load_returns_none_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(PendingActivation::load(dir.path()).unwrap().is_none());
    }

    #[test]
    fn journal_delete_removes_file() {
        let dir = tempfile::tempdir().unwrap();
        let journal = sample_journal(dir.path());
        journal.save(dir.path()).unwrap();
        assert!(PendingActivation::path(dir.path()).exists());
        PendingActivation::delete(dir.path()).unwrap();
        assert!(!PendingActivation::path(dir.path()).exists());
    }

    #[test]
    fn journal_status_update() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = sample_journal(dir.path());
        journal.save(dir.path()).unwrap();

        // Simulate update to registered
        journal.entries[0].status = PendingStatus::Registered;
        journal.save(dir.path()).unwrap();

        let loaded = PendingActivation::load(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.entries[0].status, PendingStatus::Registered);
    }

    #[test]
    fn journal_identity_match() {
        let dir = tempfile::tempdir().unwrap();
        let journal = sample_journal(dir.path());
        let reg = dir
            .path()
            .join("plugins.msgpackz")
            .to_string_lossy()
            .to_string();
        assert!(journal.matches_nu_identity("abc123", "0.113.1", &reg));
        assert!(!journal.matches_nu_identity("wrong_hash", "0.113.1", &reg));
        assert!(!journal.matches_nu_identity("abc123", "0.999.0", &reg));
        assert!(!journal.matches_nu_identity("abc123", "0.113.1", "/wrong/path"));
    }

    #[test]
    fn journal_delete_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        // Delete when no file exists — should not error
        PendingActivation::delete(dir.path()).unwrap();
    }
}
