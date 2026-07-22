use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::util::atomic::write_json_atomic;

/// Stage of an individual plugin unregistration attempt.
///
/// Lifecycle:
///   `prepared`     — journal written, `plugin rm` not yet called
///   `unregistered` — `plugin rm` exited 0, lockfile clear pending
///   `failed`       — `plugin rm` returned non-zero
///
/// After `unregistered`, the caller clears `PluginActivation` on the lockfile
/// entry, then removes this journal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PluginDeactivateStatus {
    Prepared,
    Unregistered,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingPluginDeactivateEntry {
    pub package_id: String,
    pub plugin_name: String,
    /// Resolved absolute path to the plugin binary (audit trail).
    pub absolute_binary_path: String,
    pub status: PluginDeactivateStatus,
    #[serde(default)]
    pub error: Option<String>,
}

/// Pending-plugin-deactivate journal at `<root>/state/pending-plugin-deactivate.json`.
///
/// Existence after a command exits indicates an interrupted run.
/// `numan deactivate` reconciles it on the next invocation.
/// `numan doctor` reports the journal; with `--fix`, deactivate reconciles it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingPluginDeactivate {
    pub nu_executable_sha256: String,
    pub nu_version: String,
    pub plugin_registry_path: String,
    pub created_at: String,
    pub entries: Vec<PendingPluginDeactivateEntry>,
}

impl PendingPluginDeactivate {
    fn path(root: &Path) -> PathBuf {
        root.join("state/pending-plugin-deactivate.json")
    }

    pub fn load(root: &Path) -> Result<Option<Self>> {
        let path = Self::path(root);
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read journal at '{}'", path.display()))?;
        let journal: Self = serde_json::from_str(&content)
            .context("Failed to parse pending-plugin-deactivate journal")?;
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

    fn sample_journal(root: &Path) -> PendingPluginDeactivate {
        PendingPluginDeactivate {
            nu_executable_sha256: "abc123".to_string(),
            nu_version: "0.113.1".to_string(),
            plugin_registry_path: root.join("plugins.msgpackz").to_string_lossy().to_string(),
            created_at: "0000000000000001".to_string(),
            entries: vec![PendingPluginDeactivateEntry {
                package_id: "owner/plugin".to_string(),
                plugin_name: "highlight".to_string(),
                absolute_binary_path:
                    "/numan/packages/plugins/owner/plugin/1.0.0-abc/nu_plugin_highlight"
                        .to_string(),
                status: PluginDeactivateStatus::Prepared,
                error: None,
            }],
        }
    }

    #[test]
    fn journal_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let journal = sample_journal(dir.path());
        journal.save(dir.path()).unwrap();
        let loaded = PendingPluginDeactivate::load(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.nu_executable_sha256, "abc123");
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].status, PluginDeactivateStatus::Prepared);
        assert_eq!(loaded.entries[0].plugin_name, "highlight");
    }

    #[test]
    fn journal_load_returns_none_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(PendingPluginDeactivate::load(dir.path()).unwrap().is_none());
    }

    #[test]
    fn journal_delete_removes_file() {
        let dir = tempfile::tempdir().unwrap();
        let journal = sample_journal(dir.path());
        journal.save(dir.path()).unwrap();
        assert!(PendingPluginDeactivate::path(dir.path()).exists());
        PendingPluginDeactivate::delete(dir.path()).unwrap();
        assert!(!PendingPluginDeactivate::path(dir.path()).exists());
    }

    #[test]
    fn journal_status_update() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = sample_journal(dir.path());
        journal.save(dir.path()).unwrap();

        journal.entries[0].status = PluginDeactivateStatus::Unregistered;
        journal.save(dir.path()).unwrap();

        let loaded = PendingPluginDeactivate::load(dir.path()).unwrap().unwrap();
        assert_eq!(
            loaded.entries[0].status,
            PluginDeactivateStatus::Unregistered
        );
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
        PendingPluginDeactivate::delete(dir.path()).unwrap();
    }
}
