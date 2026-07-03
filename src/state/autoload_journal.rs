use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::util::atomic::write_json_atomic;

/// Schema version for `pending-autoload.json`.
pub const SCHEMA_VERSION: u32 = 1;

/// Which high-level operation is in progress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoloadOperation {
    /// Activating one or more modules (generating/replacing `numan.nu`).
    Activate,
    /// Deactivating one or more modules (regenerating or deleting `numan.nu`).
    Deactivate,
    /// Re-validating the existing managed file after `numan init --refresh`.
    RevalidateAfterRefresh,
}

/// Durable stage of the module-autoload transaction.
///
/// Lifecycle:
///   `Prepared`  — candidate generated and journal written; `numan.nu` not yet replaced.
///   `Replaced`  — `numan.nu` replaced; lockfile and autoload-state updates pending.
///
/// After `Replaced`, the caller must update the lockfile module activation records,
/// write the derived `autoload-state.json`, and then clear the journal.
/// If any of those steps fail, the `Replaced` journal is preserved so that
/// `reconcile` can complete the work on the next invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoloadStage {
    /// Candidate generated; managed file not yet replaced.
    Prepared,
    /// Managed file replaced; lockfile and autoload-state updates pending.
    Replaced,
}

/// Module-autoload transaction journal.
///
/// Written to `$NUMAN_ROOT/state/pending-autoload.json`. Existence of this
/// file after a command exits indicates an interrupted run. `reconcile` is
/// called by `numan activate` and `numan deactivate` before any new mutation.
///
/// This journal is separate from `state/pending-activation.json` (plugins).
/// Do not merge them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingAutoload {
    pub schema_version: u32,
    /// Which operation was in progress.
    pub operation: AutoloadOperation,
    /// How far the operation had progressed.
    pub stage: AutoloadStage,

    /// SHA-256 of the Nu executable at the time the journal was written.
    pub nu_executable_sha256: String,
    /// Version string of the Nu executable at the time the journal was written.
    pub nu_version: String,
    /// Absolute path to the selected vendor-autoload directory.
    pub vendor_autoload_dir: String,
    /// Absolute path to the Numan-managed autoload file (`numan.nu`).
    pub managed_file_path: String,

    /// Was a `numan.nu` present before this operation began?
    pub previous_file_exists: bool,
    /// SHA-256 of the previous managed file, if it existed.
    pub previous_file_sha256: Option<String>,

    /// Should a `numan.nu` exist after this operation completes?
    pub desired_file_exists: bool,
    /// SHA-256 of the validated candidate that was (or will be) placed as `numan.nu`.
    pub candidate_sha256: Option<String>,

    /// Module IDs that were active before this operation began (sorted).
    pub previous_active_module_ids: Vec<String>,
    /// Module IDs that should be active after this operation completes (sorted).
    pub desired_active_module_ids: Vec<String>,

    /// The specific module IDs that this operation was targeting (sorted).
    ///
    /// For activation: the IDs being activated in this transaction.
    /// For deactivation: the IDs being deactivated in this transaction.
    pub targeted_module_ids: Vec<String>,

    /// Timestamp when the journal was first written.
    pub created_at: String,
    /// Snapshot ID created before this mutation began. Used for rollback and
    /// recovery boundaries.
    #[serde(default)]
    pub pre_mutation_snapshot_id: Option<String>,
}

impl PendingAutoload {
    fn journal_path(root: &Path) -> PathBuf {
        root.join("state/pending-autoload.json")
    }

    /// Load the journal from disk, returning `None` when absent.
    pub fn load(root: &Path) -> Result<Option<Self>> {
        let path = Self::journal_path(root);
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path).with_context(|| {
            format!(
                "Failed to read pending-autoload journal at '{}'",
                path.display()
            )
        })?;
        let journal: Self =
            serde_json::from_str(&content).context("Failed to parse pending-autoload.json")?;
        Ok(Some(journal))
    }

    /// Atomically write the journal to disk.
    pub fn save(&self, root: &Path) -> Result<()> {
        write_json_atomic(&Self::journal_path(root), self)
    }

    /// Delete (clear) the journal.
    ///
    /// Called after a transaction completes successfully or after a
    /// `Prepared` recovery confirms no external replacement occurred.
    /// Idempotent: does not error when the file is absent.
    pub fn delete(root: &Path) -> Result<()> {
        let path = Self::journal_path(root);
        if path.exists() {
            std::fs::remove_file(&path).with_context(|| {
                format!(
                    "Failed to remove pending-autoload journal at '{}'",
                    path.display()
                )
            })?;
        }
        Ok(())
    }

    /// Returns `true` when the journal's Nu identity matches the provided values.
    ///
    /// A mismatch indicates the journal is stale (Nu was updated between the
    /// interrupted run and this invocation). Stale journals block mutation and
    /// require `numan init --refresh`.
    pub fn matches_nu_identity(&self, nu_executable_sha256: &str, nu_version: &str) -> bool {
        self.nu_executable_sha256 == nu_executable_sha256 && self.nu_version == nu_version
    }

    /// Inspect a `Prepared` journal and determine whether the interrupted
    /// operation can be safely abandoned (i.e. no external file replacement
    /// occurred).
    ///
    /// Checks:
    /// 1. The journal stage must be `Prepared`.
    /// 2. The current state of the managed file must match `previous_file_exists`
    ///    and, if the file existed, its SHA-256 must match `previous_file_sha256`.
    ///
    /// # Outcomes
    ///
    /// - `Ok(RecoveryAction::AbandonedSafely)` — no replacement happened; the
    ///   journal can be cleared and mutation can proceed.
    /// - `Ok(RecoveryAction::DriftDetected)` — the managed file no longer
    ///   matches the previously-recorded state; mutation is blocked.
    /// - `Err(_)` — an I/O or parsing error prevented the check.
    pub fn recover_prepared(&self) -> Result<RecoveryAction> {
        if self.stage != AutoloadStage::Prepared {
            bail!(
                "recover_prepared called on a journal with stage {:?}; expected Prepared",
                self.stage
            );
        }

        let path = Path::new(&self.managed_file_path);
        let current_exists = path.exists();

        if current_exists != self.previous_file_exists {
            return Ok(RecoveryAction::DriftDetected {
                reason: format!(
                    "managed file existence changed: journal expected exists={}, actual={}",
                    self.previous_file_exists, current_exists
                ),
            });
        }

        if current_exists {
            // File existed before and still exists — verify SHA-256 matches.
            let expected_sha = self.previous_file_sha256.as_deref().unwrap_or("");
            let actual_sha = sha256_file(path)?;
            if actual_sha != expected_sha {
                return Ok(RecoveryAction::DriftDetected {
                    reason: format!(
                        "managed file SHA-256 changed: journal expected {}, actual {}",
                        expected_sha, actual_sha
                    ),
                });
            }
        }

        Ok(RecoveryAction::AbandonedSafely)
    }

    /// Inspect a `Replaced` journal and determine whether the lockfile and
    /// autoload-state can be updated to complete the interrupted transaction.
    ///
    /// Checks:
    /// 1. The journal stage must be `Replaced`.
    /// 2. The managed file must currently exist (replacement already happened).
    /// 3. Its SHA-256 must match `candidate_sha256`.
    ///
    /// Returns `RecoveryAction::CanComplete` when all checks pass, indicating
    /// that the caller should write lockfile activation records, write
    /// `autoload-state.json`, and then clear the journal.
    ///
    /// Returns `RecoveryAction::DriftDetected` when verification fails.
    pub fn recover_replaced(&self) -> Result<RecoveryAction> {
        if self.stage != AutoloadStage::Replaced {
            bail!(
                "recover_replaced called on a journal with stage {:?}; expected Replaced",
                self.stage
            );
        }

        let path = Path::new(&self.managed_file_path);

        if !self.desired_file_exists {
            // Deactivation path: managed file should have been deleted.
            if path.exists() {
                return Ok(RecoveryAction::DriftDetected {
                    reason: "managed file still exists after deletion was recorded as complete"
                        .to_string(),
                });
            }
            return Ok(RecoveryAction::CanComplete);
        }

        // Activation path: managed file should have been replaced.
        if !path.exists() {
            return Ok(RecoveryAction::DriftDetected {
                reason: "managed file missing after replacement was recorded as complete"
                    .to_string(),
            });
        }

        let expected_sha = self.candidate_sha256.as_deref().unwrap_or("");
        let actual_sha = sha256_file(path)?;
        if actual_sha != expected_sha {
            return Ok(RecoveryAction::DriftDetected {
                reason: format!(
                    "managed file SHA-256 mismatch after replacement: journal expected {}, actual {}",
                    expected_sha, actual_sha
                ),
            });
        }

        Ok(RecoveryAction::CanComplete)
    }
}

/// Outcome returned by the `recover_*` methods.
#[derive(Debug, PartialEq)]
pub enum RecoveryAction {
    /// `Prepared` journal: no external change occurred; clear the journal and proceed.
    AbandonedSafely,
    /// `Replaced` journal: managed file is in the expected post-replacement state;
    /// finish lockfile and autoload-state updates, then clear the journal.
    CanComplete,
    /// Managed-file drift was detected; mutation is blocked.
    DriftDetected { reason: String },
}

/// Compute the SHA-256 hex digest of a file on disk.
pub fn sha256_file(path: &Path) -> Result<String> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)
        .with_context(|| format!("Failed to open '{}' for SHA-256", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file
            .read(&mut buf)
            .with_context(|| format!("Failed to read '{}' for SHA-256", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finish())
}

/// Minimal SHA-256 implementation using the `sha2` crate (already a dependency
/// via the `integrity` module). We wrap the hasher in a small struct so the
/// call sites above remain readable without importing SHA-specific types.
struct Sha256 {
    inner: sha2::Sha256,
}

impl Sha256 {
    fn new() -> Self {
        use sha2::Digest;
        Self {
            inner: sha2::Sha256::new(),
        }
    }

    fn update(&mut self, data: &[u8]) {
        use sha2::Digest;
        self.inner.update(data);
    }

    fn finish(self) -> String {
        use sha2::Digest;
        format!("{:x}", self.inner.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_journal() -> PendingAutoload {
        PendingAutoload {
            schema_version: SCHEMA_VERSION,
            operation: AutoloadOperation::Activate,
            stage: AutoloadStage::Prepared,
            nu_executable_sha256: "exe-hash".to_string(),
            nu_version: "0.113.1".to_string(),
            vendor_autoload_dir: "/nu/vendor/autoload".to_string(),
            managed_file_path: "/nu/vendor/autoload/numan.nu".to_string(),
            previous_file_exists: false,
            previous_file_sha256: None,
            desired_file_exists: true,
            candidate_sha256: Some("candidate-sha256".to_string()),
            previous_active_module_ids: vec![],
            desired_active_module_ids: vec!["owner/foo".to_string()],
            targeted_module_ids: vec!["owner/foo".to_string()],
            created_at: "0000000000000001".to_string(),
            pre_mutation_snapshot_id: None,
        }
    }

    // ── save / load / delete ─────────────────────────────────────────────────

    #[test]
    fn roundtrip_save_load() {
        let dir = tempfile::tempdir().unwrap();
        let j = sample_journal();
        j.save(dir.path()).unwrap();
        let loaded = PendingAutoload::load(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.operation, AutoloadOperation::Activate);
        assert_eq!(loaded.stage, AutoloadStage::Prepared);
        assert_eq!(loaded.nu_executable_sha256, "exe-hash");
        assert_eq!(loaded.desired_active_module_ids, vec!["owner/foo"]);
    }

    #[test]
    fn load_returns_none_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(PendingAutoload::load(dir.path()).unwrap().is_none());
    }

    #[test]
    fn delete_removes_file() {
        let dir = tempfile::tempdir().unwrap();
        let j = sample_journal();
        j.save(dir.path()).unwrap();
        assert!(PendingAutoload::journal_path(dir.path()).exists());
        PendingAutoload::delete(dir.path()).unwrap();
        assert!(!PendingAutoload::journal_path(dir.path()).exists());
    }

    #[test]
    fn delete_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        PendingAutoload::delete(dir.path()).unwrap();
    }

    // ── stage update survives roundtrip ──────────────────────────────────────

    #[test]
    fn stage_update_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut j = sample_journal();
        j.save(dir.path()).unwrap();

        j.stage = AutoloadStage::Replaced;
        j.save(dir.path()).unwrap();

        let loaded = PendingAutoload::load(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.stage, AutoloadStage::Replaced);
    }

    // ── matches_nu_identity ──────────────────────────────────────────────────

    #[test]
    fn nu_identity_matches_correctly() {
        let j = sample_journal();
        assert!(j.matches_nu_identity("exe-hash", "0.113.1"));
        assert!(!j.matches_nu_identity("wrong-hash", "0.113.1"));
        assert!(!j.matches_nu_identity("exe-hash", "0.114.0"));
    }

    // ── recover_prepared ────────────────────────────────────────────────────

    #[test]
    fn recover_prepared_safe_when_file_still_absent() {
        let dir = tempfile::tempdir().unwrap();
        let managed = dir.path().join("numan.nu");
        let mut j = sample_journal();
        j.managed_file_path = managed.to_string_lossy().into_owned();
        j.previous_file_exists = false;
        // managed file does not exist — matches journal expectation
        let action = j.recover_prepared().unwrap();
        assert_eq!(action, RecoveryAction::AbandonedSafely);
    }

    #[test]
    fn recover_prepared_drift_when_file_appeared() {
        let dir = tempfile::tempdir().unwrap();
        let managed = dir.path().join("numan.nu");
        std::fs::write(&managed, b"# something").unwrap();
        let mut j = sample_journal();
        j.managed_file_path = managed.to_string_lossy().into_owned();
        j.previous_file_exists = false; // journal says it shouldn't exist
        let action = j.recover_prepared().unwrap();
        assert!(matches!(action, RecoveryAction::DriftDetected { .. }));
    }

    #[test]
    fn recover_prepared_safe_when_file_sha_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let managed = dir.path().join("numan.nu");
        let content =
            b"# Generated and managed by Numan. Do not edit.\n# Numan autoload schema: 1\n";
        std::fs::write(&managed, content).unwrap();
        let sha = sha256_file(&managed).unwrap();

        let mut j = sample_journal();
        j.managed_file_path = managed.to_string_lossy().into_owned();
        j.previous_file_exists = true;
        j.previous_file_sha256 = Some(sha);

        let action = j.recover_prepared().unwrap();
        assert_eq!(action, RecoveryAction::AbandonedSafely);
    }

    #[test]
    fn recover_prepared_drift_when_file_sha_changed() {
        let dir = tempfile::tempdir().unwrap();
        let managed = dir.path().join("numan.nu");
        std::fs::write(&managed, b"modified content").unwrap();

        let mut j = sample_journal();
        j.managed_file_path = managed.to_string_lossy().into_owned();
        j.previous_file_exists = true;
        j.previous_file_sha256 = Some("old-sha256".to_string());

        let action = j.recover_prepared().unwrap();
        assert!(matches!(action, RecoveryAction::DriftDetected { .. }));
    }

    #[test]
    fn recover_prepared_errors_when_stage_is_replaced() {
        let mut j = sample_journal();
        j.stage = AutoloadStage::Replaced;
        assert!(j.recover_prepared().is_err());
    }

    // ── recover_replaced ────────────────────────────────────────────────────

    #[test]
    fn recover_replaced_can_complete_when_candidate_sha_matches() {
        let dir = tempfile::tempdir().unwrap();
        let managed = dir.path().join("numan.nu");
        let content =
            b"# Generated and managed by Numan. Do not edit.\n# Numan autoload schema: 1\n";
        std::fs::write(&managed, content).unwrap();
        let sha = sha256_file(&managed).unwrap();

        let mut j = sample_journal();
        j.stage = AutoloadStage::Replaced;
        j.managed_file_path = managed.to_string_lossy().into_owned();
        j.desired_file_exists = true;
        j.candidate_sha256 = Some(sha);

        let action = j.recover_replaced().unwrap();
        assert_eq!(action, RecoveryAction::CanComplete);
    }

    #[test]
    fn recover_replaced_drift_when_candidate_sha_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let managed = dir.path().join("numan.nu");
        std::fs::write(&managed, b"tampered content").unwrap();

        let mut j = sample_journal();
        j.stage = AutoloadStage::Replaced;
        j.managed_file_path = managed.to_string_lossy().into_owned();
        j.desired_file_exists = true;
        j.candidate_sha256 = Some("expected-sha".to_string());

        let action = j.recover_replaced().unwrap();
        assert!(matches!(action, RecoveryAction::DriftDetected { .. }));
    }

    #[test]
    fn recover_replaced_drift_when_file_missing_after_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let managed = dir.path().join("numan.nu"); // not created

        let mut j = sample_journal();
        j.stage = AutoloadStage::Replaced;
        j.managed_file_path = managed.to_string_lossy().into_owned();
        j.desired_file_exists = true;
        j.candidate_sha256 = Some("some-sha".to_string());

        let action = j.recover_replaced().unwrap();
        assert!(matches!(action, RecoveryAction::DriftDetected { .. }));
    }

    #[test]
    fn recover_replaced_can_complete_for_deactivation_when_file_absent() {
        let dir = tempfile::tempdir().unwrap();
        let managed = dir.path().join("numan.nu"); // not created

        let mut j = sample_journal();
        j.operation = AutoloadOperation::Deactivate;
        j.stage = AutoloadStage::Replaced;
        j.managed_file_path = managed.to_string_lossy().into_owned();
        j.desired_file_exists = false; // full deactivation
        j.candidate_sha256 = None;

        let action = j.recover_replaced().unwrap();
        assert_eq!(action, RecoveryAction::CanComplete);
    }

    #[test]
    fn recover_replaced_drift_for_deactivation_when_file_still_exists() {
        let dir = tempfile::tempdir().unwrap();
        let managed = dir.path().join("numan.nu");
        std::fs::write(&managed, b"# still here").unwrap();

        let mut j = sample_journal();
        j.operation = AutoloadOperation::Deactivate;
        j.stage = AutoloadStage::Replaced;
        j.managed_file_path = managed.to_string_lossy().into_owned();
        j.desired_file_exists = false;
        j.candidate_sha256 = None;

        let action = j.recover_replaced().unwrap();
        assert!(matches!(action, RecoveryAction::DriftDetected { .. }));
    }

    #[test]
    fn recover_replaced_errors_when_stage_is_prepared() {
        let j = sample_journal(); // stage = Prepared
        assert!(j.recover_replaced().is_err());
    }

    // ── operation and stage serde ────────────────────────────────────────────

    #[test]
    fn operation_serde_roundtrip() {
        let ops = [
            AutoloadOperation::Activate,
            AutoloadOperation::Deactivate,
            AutoloadOperation::RevalidateAfterRefresh,
        ];
        for op in &ops {
            let s = serde_json::to_string(op).unwrap();
            let parsed: AutoloadOperation = serde_json::from_str(&s).unwrap();
            assert_eq!(&parsed, op);
        }
    }

    #[test]
    fn stage_serde_roundtrip() {
        let stages = [AutoloadStage::Prepared, AutoloadStage::Replaced];
        for stage in &stages {
            let s = serde_json::to_string(stage).unwrap();
            let parsed: AutoloadStage = serde_json::from_str(&s).unwrap();
            assert_eq!(&parsed, stage);
        }
    }
}
