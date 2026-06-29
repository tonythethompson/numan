use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::util::atomic::write_json_atomic;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleOp {
    Update,
    Remove,
}

/// Stage of the in-flight lifecycle operation at the time the process last
/// checkpointed. Used for crash detection on the next invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleStage {
    /// Journal written; no mutations have been applied yet.
    Prepared,
    /// Lockfile has been updated; payload directory deletion may be pending.
    LockfileUpdated,
}

/// Crash-recovery journal written atomically before any destructive lifecycle
/// mutation (update, remove).
///
/// Written before mutations begin; cleared on successful completion. A journal
/// found on the next run indicates an interrupted operation. `numan gc` can
/// clean up any orphaned payload directories.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingLifecycle {
    pub op: LifecycleOp,
    pub package_id: String,
    pub stage: LifecycleStage,
    /// Payload directory (relative to root) that is orphaned or being removed.
    #[serde(default)]
    pub orphan_payload_path: Option<String>,
    /// For `update`: the version we upgraded from.
    #[serde(default)]
    pub from_version: Option<String>,
    /// For `update`: the version we upgraded to.
    #[serde(default)]
    pub to_version: Option<String>,
}

impl PendingLifecycle {
    fn path(root: &Path) -> std::path::PathBuf {
        root.join("state/pending-lifecycle.json")
    }

    pub fn load(root: &Path) -> Result<Option<Self>> {
        let p = Self::path(root);
        if !p.exists() {
            return Ok(None);
        }
        let s = std::fs::read_to_string(&p)
            .with_context(|| format!("Failed to read {}", p.display()))?;
        Ok(Some(serde_json::from_str(&s)?))
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        write_json_atomic(&Self::path(root), self)
    }

    pub fn clear(root: &Path) -> Result<()> {
        let p = Self::path(root);
        if p.exists() {
            std::fs::remove_file(&p)
                .with_context(|| format!("Failed to remove {}", p.display()))?;
        }
        Ok(())
    }
}

/// Check for a stale lifecycle journal and return it if present.
///
/// Callers should warn the user and suggest `numan gc` when `Some` is returned.
pub fn check_stale_journal(root: &Path) -> Result<Option<PendingLifecycle>> {
    PendingLifecycle::load(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_remove_journal() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let j = PendingLifecycle {
            op: LifecycleOp::Remove,
            package_id: "owner/pkg".to_string(),
            stage: LifecycleStage::Prepared,
            orphan_payload_path: Some("packages/modules/owner/pkg/1.0.0-abc".to_string()),
            from_version: None,
            to_version: None,
        };
        j.save(root).unwrap();

        let loaded = PendingLifecycle::load(root).unwrap().unwrap();
        assert_eq!(loaded.package_id, "owner/pkg");
        assert!(loaded.from_version.is_none());

        PendingLifecycle::clear(root).unwrap();
        assert!(PendingLifecycle::load(root).unwrap().is_none());
    }

    #[test]
    fn roundtrip_update_journal() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let j = PendingLifecycle {
            op: LifecycleOp::Update,
            package_id: "owner/pkg".to_string(),
            stage: LifecycleStage::Prepared,
            orphan_payload_path: Some("packages/modules/owner/pkg/1.0.0-abc".to_string()),
            from_version: Some("1.0.0".to_string()),
            to_version: Some("2.0.0".to_string()),
        };
        j.save(root).unwrap();

        let loaded = PendingLifecycle::load(root).unwrap().unwrap();
        assert_eq!(loaded.from_version.as_deref(), Some("1.0.0"));
        assert_eq!(loaded.to_version.as_deref(), Some("2.0.0"));
    }

    #[test]
    fn load_missing_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(PendingLifecycle::load(dir.path()).unwrap().is_none());
    }

    #[test]
    fn check_stale_journal_returns_none_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(check_stale_journal(dir.path()).unwrap().is_none());
    }
}
