# Autoload Journal Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make module-autoload recovery command-independent so interrupted activation or deactivation reaches the same consistent lockfile, managed-file, and derived-state result regardless of which mutating command runs next.

**Architecture:** Add `state::autoload_recovery` as the single owner of applying a verified `PendingAutoload` previous-to-desired transition. Both commands reconcile and resolve targets under a planning mutation lock, release the lock for consent, then repeat reconciliation and target resolution under the mutation lock before starting new work.

**Tech Stack:** Rust 2021, MSRV 1.88, `anyhow`, `serde`, existing SHA-256 and atomic-write utilities, Rust unit/integration tests.

## Global Constraints

- Do not change the `PendingAutoload` JSON schema or durable `Prepared` / `Replaced` stages.
- The lockfile remains authoritative; `autoload-state.json` remains a derived projection.
- Journal deletion is the final recovery commit marker.
- All recovery calls from commands occur while `acquire_mutation_lock(root)` is held.
- Do not hold the mutation lock while waiting for interactive consent.
- Do not change plugin recovery semantics, CLI flags, candidate generation, snapshots, or managed-file ownership rules.
- Do not add dependencies; keep Rust `rust-version = "1.88"` and edition 2021.
- Use `&Path`, not `&PathBuf`, in function parameters.
- Tests use `FakeCandidateRunner` and injectable registrars; do not spawn a real Nu binary.
- Follow red-green-refactor: observe every new regression test fail for the expected reason before production edits.

---

### Task 1: Reproduce caller-dependent recovery failures

**Files:**
- Modify: `tests/module_autoload_test.rs:30-35,723-861`

**Interfaces:**
- Consumes: `run_activate`, `run_deactivate`, `ModuleTestEnv`, `PendingAutoload`, `AutoloadStage`, `AutoloadOperation`, `sha256_file`.
- Produces: two command-level regression tests that fail against the current duplicated reconcilers.

- [ ] **Step 1: Extend the journal imports**

Replace the existing autoload-journal import with:

```rust
use numan_cli::state::autoload_journal::{
    sha256_file, AutoloadOperation, AutoloadStage, PendingAutoload,
    SCHEMA_VERSION as AUTOLOAD_SCHEMA_VERSION,
};
```

- [ ] **Step 2: Write the interrupted-activation-followed-by-deactivation regression**

Append this test before the concurrency-lock section:

```rust
#[test]
fn deactivate_recovers_replaced_activation_before_classifying_targets() {
    let env = ModuleTestEnv::new();
    env.write_nu_paths();

    let (pkg_id, entry) =
        env.create_module("owner", "recover-activate", "1.0.0", ModuleImportMode::Module);
    let entry_path = env.root().join(&entry.payload_path).join("mod.nu");
    let mut lockfile = Lockfile::empty();
    lockfile.packages.insert(pkg_id.clone(), entry);
    env.write_lockfile(&lockfile);

    let managed_content = format!(
        "{}\nuse \"{}\"\n",
        OWNERSHIP_MARKER,
        entry_path.display()
    );
    std::fs::write(env.managed_file_path(), managed_content).unwrap();

    PendingAutoload {
        schema_version: AUTOLOAD_SCHEMA_VERSION,
        operation: AutoloadOperation::Activate,
        stage: AutoloadStage::Replaced,
        nu_executable_sha256: env.nu_hash.clone(),
        nu_version: "0.113.1".to_string(),
        vendor_autoload_dir: env.vendor_dir_str(),
        managed_file_path: env.managed_file_path_str(),
        previous_file_exists: false,
        previous_file_sha256: None,
        desired_file_exists: true,
        candidate_sha256: Some(sha256_file(&env.managed_file_path()).unwrap()),
        previous_active_module_ids: vec![],
        desired_active_module_ids: vec![pkg_id.clone()],
        targeted_module_ids: vec![pkg_id.clone()],
        created_at: "0000000000000001".to_string(),
        pre_mutation_snapshot_id: None,
    }
    .save(env.root())
    .unwrap();

    let runner = FakeCandidateRunner::success();
    run_deactivate(&env, &[&pkg_id], &runner).unwrap();

    let recovered = Lockfile::load(env.root()).unwrap();
    assert!(recovered.packages[&pkg_id].module_activation.is_none());
    assert!(!env.managed_file_path().exists());
    assert!(PendingAutoload::load(env.root()).unwrap().is_none());
}
```

- [ ] **Step 3: Write the interrupted-deactivation-followed-by-activation regression**

Append this second test:

```rust
#[test]
fn activate_recovers_full_deactivation_without_reactivating_removed_module() {
    let env = ModuleTestEnv::new();
    env.write_nu_paths();

    let (old_id, old_entry) =
        env.create_module("owner", "old", "1.0.0", ModuleImportMode::Module);
    let (next_id, next_entry) =
        env.create_module("owner", "next", "1.0.0", ModuleImportMode::Module);
    let mut lockfile = Lockfile::empty();
    lockfile.packages.insert(old_id.clone(), old_entry);
    lockfile.packages.insert(next_id.clone(), next_entry);
    env.write_lockfile(&lockfile);

    let runner = FakeCandidateRunner::success();
    run_activate(&env, &[&old_id], &runner).unwrap();
    let previous_sha = sha256_file(&env.managed_file_path()).unwrap();
    std::fs::remove_file(env.managed_file_path()).unwrap();

    PendingAutoload {
        schema_version: AUTOLOAD_SCHEMA_VERSION,
        operation: AutoloadOperation::Deactivate,
        stage: AutoloadStage::Replaced,
        nu_executable_sha256: env.nu_hash.clone(),
        nu_version: "0.113.1".to_string(),
        vendor_autoload_dir: env.vendor_dir_str(),
        managed_file_path: env.managed_file_path_str(),
        previous_file_exists: true,
        previous_file_sha256: Some(previous_sha),
        desired_file_exists: false,
        candidate_sha256: None,
        previous_active_module_ids: vec![old_id.clone()],
        desired_active_module_ids: vec![],
        targeted_module_ids: vec![old_id.clone()],
        created_at: "0000000000000002".to_string(),
        pre_mutation_snapshot_id: None,
    }
    .save(env.root())
    .unwrap();

    run_activate(&env, &[&next_id], &runner).unwrap();

    let recovered = Lockfile::load(env.root()).unwrap();
    assert!(recovered.packages[&old_id].module_activation.is_none());
    assert!(recovered.packages[&next_id].module_activation.is_some());
    assert_eq!(
        AutoloadState::load(env.root())
            .unwrap()
            .unwrap()
            .active_module_ids,
        vec![next_id]
    );
    assert!(PendingAutoload::load(env.root()).unwrap().is_none());
}
```

- [ ] **Step 4: Run the regressions and verify RED**

Run:

```bash
cargo test --test module_autoload_test deactivate_recovers_replaced_activation_before_classifying_targets -- --exact
cargo test --test module_autoload_test activate_recovers_full_deactivation_without_reactivating_removed_module -- --exact
```

Expected:

- The first test fails because `deactivate` reports that the module is not currently active before journal recovery.
- The second test fails because activation-side recovery leaves `old_id.module_activation` populated and includes it in the new managed state.

- [ ] **Step 5: Commit only after Task 2 makes these tests green**

Do not commit failing tests. They are committed together with the shared recovery implementation in Task 2.

---

### Task 2: Add the shared recovery engine and wire both commands

**Files:**
- Create: `src/state/autoload_recovery.rs`
- Modify: `src/state/mod.rs:1-8`
- Modify: `src/cmd/activate.rs:1-16,101-256,1040-1270`
- Modify: `src/cmd/deactivate.rs:1-18,55-225,655-809`
- Modify: `tests/module_autoload_test.rs`

**Interfaces:**
- Consumes: `PendingAutoload::{load,delete,recover_prepared,recover_replaced}`, `NuPaths`, `Lockfile`, `ModuleActivation`, `AutoloadState`, `sha256_file`.
- Produces: `reconcile_pending_autoload(root: &Path, nu_paths: &NuPaths, lockfile: &mut Lockfile) -> Result<AutoloadRecoveryOutcome>`.

- [ ] **Step 1: Add direct recovery tests before declaring the module**

Add tests in `tests/module_autoload_test.rs` that import the future interface and `ModuleActivation`:

```rust
use numan_cli::state::autoload_recovery::{
    reconcile_pending_autoload, AutoloadRecoveryOutcome,
};
use numan_cli::state::lockfile::ModuleActivation;
```

Append these complete tests:

```rust
#[test]
fn prepared_recovery_clears_unchanged_journal() {
    let env = ModuleTestEnv::new();
    env.write_nu_paths();
    env.write_lockfile(&Lockfile::empty());
    std::fs::create_dir_all(env.root().join("state")).unwrap();

    PendingAutoload {
        schema_version: AUTOLOAD_SCHEMA_VERSION,
        operation: AutoloadOperation::Activate,
        stage: AutoloadStage::Prepared,
        nu_executable_sha256: env.nu_hash.clone(),
        nu_version: "0.113.1".to_string(),
        vendor_autoload_dir: env.vendor_dir_str(),
        managed_file_path: env.managed_file_path_str(),
        previous_file_exists: false,
        previous_file_sha256: None,
        desired_file_exists: true,
        candidate_sha256: None,
        previous_active_module_ids: vec![],
        desired_active_module_ids: vec![],
        targeted_module_ids: vec![],
        created_at: "0000000000000003".to_string(),
        pre_mutation_snapshot_id: None,
    }
    .save(env.root())
    .unwrap();

    let _guard = acquire_mutation_lock(env.root()).unwrap();
    let nu_paths = NuPaths::load(env.root()).unwrap();
    let mut lockfile = Lockfile::load(env.root()).unwrap();
    let outcome =
        reconcile_pending_autoload(env.root(), &nu_paths, &mut lockfile).unwrap();

    assert_eq!(outcome, AutoloadRecoveryOutcome::PreparedCleared);
    assert!(PendingAutoload::load(env.root()).unwrap().is_none());
}

#[test]
fn stale_identity_recovery_preserves_journal() {
    let env = ModuleTestEnv::new();
    env.write_nu_paths();
    env.write_lockfile(&Lockfile::empty());
    std::fs::create_dir_all(env.root().join("state")).unwrap();

    PendingAutoload {
        schema_version: AUTOLOAD_SCHEMA_VERSION,
        operation: AutoloadOperation::Activate,
        stage: AutoloadStage::Prepared,
        nu_executable_sha256: "different-hash".to_string(),
        nu_version: "0.113.1".to_string(),
        vendor_autoload_dir: env.vendor_dir_str(),
        managed_file_path: env.managed_file_path_str(),
        previous_file_exists: false,
        previous_file_sha256: None,
        desired_file_exists: true,
        candidate_sha256: None,
        previous_active_module_ids: vec![],
        desired_active_module_ids: vec![],
        targeted_module_ids: vec![],
        created_at: "0000000000000004".to_string(),
        pre_mutation_snapshot_id: None,
    }
    .save(env.root())
    .unwrap();

    let _guard = acquire_mutation_lock(env.root()).unwrap();
    let nu_paths = NuPaths::load(env.root()).unwrap();
    let mut lockfile = Lockfile::load(env.root()).unwrap();
    let error = reconcile_pending_autoload(env.root(), &nu_paths, &mut lockfile)
        .unwrap_err();

    assert!(error.to_string().contains("different Nu identity"));
    assert!(PendingAutoload::load(env.root()).unwrap().is_some());
}

#[test]
fn replaced_recovery_preserves_retargeted_activation() {
    let env = ModuleTestEnv::new();
    env.write_nu_paths();
    let (pkg_id, mut entry) =
        env.create_module("owner", "retargeted", "1.0.0", ModuleImportMode::Module);
    entry.module_activation = Some(ModuleActivation {
        entry_path: env
            .root()
            .join(&entry.payload_path)
            .join("mod.nu")
            .to_string_lossy()
            .into_owned(),
        import_mode: ModuleImportMode::Module,
        vendor_autoload_dir: env.vendor_dir_str(),
        managed_file_path: env.managed_file_path_str(),
        nu_executable_sha256: "newer-nu-hash".to_string(),
        nu_version: "0.114.0".to_string(),
        activated_at: "0000000000000005".to_string(),
    });
    let mut lockfile = Lockfile::empty();
    lockfile.packages.insert(pkg_id.clone(), entry);
    env.write_lockfile(&lockfile);
    std::fs::create_dir_all(env.root().join("state")).unwrap();

    PendingAutoload {
        schema_version: AUTOLOAD_SCHEMA_VERSION,
        operation: AutoloadOperation::Deactivate,
        stage: AutoloadStage::Replaced,
        nu_executable_sha256: env.nu_hash.clone(),
        nu_version: "0.113.1".to_string(),
        vendor_autoload_dir: env.vendor_dir_str(),
        managed_file_path: env.managed_file_path_str(),
        previous_file_exists: true,
        previous_file_sha256: Some("old-file-hash".to_string()),
        desired_file_exists: false,
        candidate_sha256: None,
        previous_active_module_ids: vec![pkg_id.clone()],
        desired_active_module_ids: vec![],
        targeted_module_ids: vec![pkg_id.clone()],
        created_at: "0000000000000006".to_string(),
        pre_mutation_snapshot_id: None,
    }
    .save(env.root())
    .unwrap();

    let _guard = acquire_mutation_lock(env.root()).unwrap();
    let nu_paths = NuPaths::load(env.root()).unwrap();
    let mut lockfile = Lockfile::load(env.root()).unwrap();
    let outcome =
        reconcile_pending_autoload(env.root(), &nu_paths, &mut lockfile).unwrap();

    assert_eq!(outcome, AutoloadRecoveryOutcome::ReplacedCompleted);
    let persisted = Lockfile::load(env.root()).unwrap();
    assert_eq!(
        persisted.packages[&pkg_id]
            .module_activation
            .as_ref()
            .unwrap()
            .nu_version,
        "0.114.0"
    );
    assert!(PendingAutoload::load(env.root()).unwrap().is_none());
}

#[test]
fn invalid_deleted_file_journal_preserves_journal() {
    let env = ModuleTestEnv::new();
    env.write_nu_paths();
    let (pkg_id, entry) =
        env.create_module("owner", "invalid", "1.0.0", ModuleImportMode::Module);
    let mut lockfile = Lockfile::empty();
    lockfile.packages.insert(pkg_id.clone(), entry);
    env.write_lockfile(&lockfile);
    std::fs::create_dir_all(env.root().join("state")).unwrap();

    PendingAutoload {
        schema_version: AUTOLOAD_SCHEMA_VERSION,
        operation: AutoloadOperation::Deactivate,
        stage: AutoloadStage::Replaced,
        nu_executable_sha256: env.nu_hash.clone(),
        nu_version: "0.113.1".to_string(),
        vendor_autoload_dir: env.vendor_dir_str(),
        managed_file_path: env.managed_file_path_str(),
        previous_file_exists: true,
        previous_file_sha256: Some("old-file-hash".to_string()),
        desired_file_exists: false,
        candidate_sha256: None,
        previous_active_module_ids: vec![pkg_id.clone()],
        desired_active_module_ids: vec![pkg_id],
        targeted_module_ids: vec![],
        created_at: "0000000000000007".to_string(),
        pre_mutation_snapshot_id: None,
    }
    .save(env.root())
    .unwrap();

    let _guard = acquire_mutation_lock(env.root()).unwrap();
    let nu_paths = NuPaths::load(env.root()).unwrap();
    let mut lockfile = Lockfile::load(env.root()).unwrap();
    let error = reconcile_pending_autoload(env.root(), &nu_paths, &mut lockfile)
        .unwrap_err();

    assert!(error.to_string().contains("cannot declare active modules"));
    assert!(PendingAutoload::load(env.root()).unwrap().is_some());
}

#[test]
fn missing_entry_metadata_preserves_replaced_journal() {
    let env = ModuleTestEnv::new();
    env.write_nu_paths();
    let (pkg_id, mut entry) =
        env.create_module("owner", "missing-entry", "1.0.0", ModuleImportMode::Module);
    entry.entry = None;
    let mut lockfile = Lockfile::empty();
    lockfile.packages.insert(pkg_id.clone(), entry);
    env.write_lockfile(&lockfile);
    let managed_content = format!("{}\n", OWNERSHIP_MARKER);
    std::fs::write(env.managed_file_path(), managed_content).unwrap();
    std::fs::create_dir_all(env.root().join("state")).unwrap();

    PendingAutoload {
        schema_version: AUTOLOAD_SCHEMA_VERSION,
        operation: AutoloadOperation::Activate,
        stage: AutoloadStage::Replaced,
        nu_executable_sha256: env.nu_hash.clone(),
        nu_version: "0.113.1".to_string(),
        vendor_autoload_dir: env.vendor_dir_str(),
        managed_file_path: env.managed_file_path_str(),
        previous_file_exists: false,
        previous_file_sha256: None,
        desired_file_exists: true,
        candidate_sha256: Some(sha256_file(&env.managed_file_path()).unwrap()),
        previous_active_module_ids: vec![],
        desired_active_module_ids: vec![pkg_id.clone()],
        targeted_module_ids: vec![pkg_id],
        created_at: "0000000000000008".to_string(),
        pre_mutation_snapshot_id: None,
    }
    .save(env.root())
    .unwrap();

    let _guard = acquire_mutation_lock(env.root()).unwrap();
    let nu_paths = NuPaths::load(env.root()).unwrap();
    let mut lockfile = Lockfile::load(env.root()).unwrap();
    let error = reconcile_pending_autoload(env.root(), &nu_paths, &mut lockfile)
        .unwrap_err();

    assert!(error.to_string().contains("entry is not set"));
    assert!(PendingAutoload::load(env.root()).unwrap().is_some());
}

#[test]
fn lockfile_save_failure_preserves_replaced_journal() {
    let env = ModuleTestEnv::new();
    env.write_nu_paths();
    let (pkg_id, entry) =
        env.create_module("owner", "lockfile-failure", "1.0.0", ModuleImportMode::Module);
    let mut initial = Lockfile::empty();
    initial.packages.insert(pkg_id.clone(), entry);
    env.write_lockfile(&initial);
    let managed_content = format!("{}\n", OWNERSHIP_MARKER);
    std::fs::write(env.managed_file_path(), managed_content).unwrap();
    std::fs::create_dir_all(env.root().join("state")).unwrap();

    PendingAutoload {
        schema_version: AUTOLOAD_SCHEMA_VERSION,
        operation: AutoloadOperation::Activate,
        stage: AutoloadStage::Replaced,
        nu_executable_sha256: env.nu_hash.clone(),
        nu_version: "0.113.1".to_string(),
        vendor_autoload_dir: env.vendor_dir_str(),
        managed_file_path: env.managed_file_path_str(),
        previous_file_exists: false,
        previous_file_sha256: None,
        desired_file_exists: true,
        candidate_sha256: Some(sha256_file(&env.managed_file_path()).unwrap()),
        previous_active_module_ids: vec![],
        desired_active_module_ids: vec![pkg_id.clone()],
        targeted_module_ids: vec![pkg_id],
        created_at: "0000000000000009".to_string(),
        pre_mutation_snapshot_id: None,
    }
    .save(env.root())
    .unwrap();

    let nu_paths = NuPaths::load(env.root()).unwrap();
    let mut lockfile = Lockfile::load(env.root()).unwrap();
    std::fs::remove_file(env.root().join("lockfile")).unwrap();
    std::fs::create_dir(env.root().join("lockfile")).unwrap();
    let _guard = acquire_mutation_lock(env.root()).unwrap();
    assert!(reconcile_pending_autoload(env.root(), &nu_paths, &mut lockfile).is_err());
    assert!(PendingAutoload::load(env.root()).unwrap().is_some());
}

#[test]
fn derived_state_failure_is_idempotent_on_retry() {
    let env = ModuleTestEnv::new();
    env.write_nu_paths();
    let (pkg_id, entry) =
        env.create_module("owner", "retry", "1.0.0", ModuleImportMode::Module);
    let mut initial = Lockfile::empty();
    initial.packages.insert(pkg_id.clone(), entry);
    env.write_lockfile(&initial);
    let managed_content = format!("{}\n", OWNERSHIP_MARKER);
    std::fs::write(env.managed_file_path(), managed_content).unwrap();
    std::fs::create_dir_all(env.root().join("state")).unwrap();

    PendingAutoload {
        schema_version: AUTOLOAD_SCHEMA_VERSION,
        operation: AutoloadOperation::Activate,
        stage: AutoloadStage::Replaced,
        nu_executable_sha256: env.nu_hash.clone(),
        nu_version: "0.113.1".to_string(),
        vendor_autoload_dir: env.vendor_dir_str(),
        managed_file_path: env.managed_file_path_str(),
        previous_file_exists: false,
        previous_file_sha256: None,
        desired_file_exists: true,
        candidate_sha256: Some(sha256_file(&env.managed_file_path()).unwrap()),
        previous_active_module_ids: vec![],
        desired_active_module_ids: vec![pkg_id.clone()],
        targeted_module_ids: vec![pkg_id.clone()],
        created_at: "0000000000000010".to_string(),
        pre_mutation_snapshot_id: None,
    }
    .save(env.root())
    .unwrap();

    let nu_paths = NuPaths::load(env.root()).unwrap();
    std::fs::remove_dir_all(env.root().join("nu_state")).unwrap();
    std::fs::write(env.root().join("nu_state"), b"blocked").unwrap();

    {
        let _guard = acquire_mutation_lock(env.root()).unwrap();
        let mut lockfile = Lockfile::load(env.root()).unwrap();
        assert!(reconcile_pending_autoload(env.root(), &nu_paths, &mut lockfile).is_err());
    }
    assert!(PendingAutoload::load(env.root()).unwrap().is_some());

    std::fs::remove_file(env.root().join("nu_state")).unwrap();
    std::fs::create_dir_all(env.root().join("nu_state")).unwrap();
    {
        let _guard = acquire_mutation_lock(env.root()).unwrap();
        let mut lockfile = Lockfile::load(env.root()).unwrap();
        let outcome =
            reconcile_pending_autoload(env.root(), &nu_paths, &mut lockfile).unwrap();
        assert_eq!(outcome, AutoloadRecoveryOutcome::ReplacedCompleted);
    }

    assert!(PendingAutoload::load(env.root()).unwrap().is_none());
    assert!(Lockfile::load(env.root()).unwrap().packages[&pkg_id]
        .module_activation
        .is_some());
    assert!(AutoloadState::load(env.root()).unwrap().is_some());
}
```

- [ ] **Step 2: Verify the direct tests fail to compile**

Run:

```bash
cargo test --test module_autoload_test prepared_recovery_clears_unchanged_journal -- --exact
```

Expected: FAIL with unresolved import `numan_cli::state::autoload_recovery`.

- [ ] **Step 3: Export the focused recovery module**

Add to `src/state/mod.rs`:

```rust
pub mod autoload_recovery;
```

- [ ] **Step 4: Implement the command-independent transition**

Create `src/state/autoload_recovery.rs` with this implementation:

```rust
use anyhow::{anyhow, bail, Result};
use std::collections::BTreeSet;
use std::path::Path;

use crate::core::package::ModuleImportMode;
use crate::nu::paths::NuPaths;
use crate::state::autoload_journal::{
    sha256_file, AutoloadStage, PendingAutoload, RecoveryAction,
};
use crate::state::autoload_state::AutoloadState;
use crate::state::lockfile::{Lockfile, LockfileEntry, ModuleActivation};
use crate::util::format_timestamp;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoloadRecoveryOutcome {
    NoJournal,
    PreparedCleared,
    ReplacedCompleted,
}

pub fn reconcile_pending_autoload(
    root: &Path,
    nu_paths: &NuPaths,
    lockfile: &mut Lockfile,
) -> Result<AutoloadRecoveryOutcome> {
    let Some(journal) = PendingAutoload::load(root)? else {
        return Ok(AutoloadRecoveryOutcome::NoJournal);
    };

    if !journal.matches_nu_identity(&nu_paths.nu_executable_hash, &nu_paths.nu_version) {
        bail!(
            "A pending module-autoload journal exists from a different Nu identity.\n\
             Run 'numan init --refresh' to clear stale state, then retry.\n\
             Journal: {}",
            root.join("state/pending-autoload.json").display()
        );
    }

    match journal.stage {
        AutoloadStage::Prepared => match journal.recover_prepared()? {
            RecoveryAction::AbandonedSafely => {
                PendingAutoload::delete(root)?;
                Ok(AutoloadRecoveryOutcome::PreparedCleared)
            }
            RecoveryAction::DriftDetected { reason } => bail!(
                "Numan managed-file drift detected during module journal recovery.\n\
                 {reason}\n\
                 Resolve the drift manually before proceeding."
            ),
            RecoveryAction::CanComplete => {
                bail!("Prepared autoload recovery returned an invalid completion action")
            }
        },
        AutoloadStage::Replaced => match journal.recover_replaced()? {
            RecoveryAction::CanComplete => {
                apply_replaced_transition(root, nu_paths, lockfile, &journal)?;
                PendingAutoload::delete(root)?;
                Ok(AutoloadRecoveryOutcome::ReplacedCompleted)
            }
            RecoveryAction::DriftDetected { reason } => bail!(
                "Cannot complete module-autoload journal recovery — drift detected.\n\
                 {reason}\n\
                 Preserve the journal and investigate manually."
            ),
            RecoveryAction::AbandonedSafely => {
                bail!("Replaced autoload recovery returned an invalid abandon action")
            }
        },
    }
}

fn apply_replaced_transition(
    root: &Path,
    nu_paths: &NuPaths,
    lockfile: &mut Lockfile,
    journal: &PendingAutoload,
) -> Result<()> {
    if !journal.desired_file_exists && !journal.desired_active_module_ids.is_empty() {
        bail!(
            "Invalid module-autoload journal: a deleted managed file cannot declare active modules"
        );
    }

    let desired: BTreeSet<&str> = journal
        .desired_active_module_ids
        .iter()
        .map(String::as_str)
        .collect();
    let activated_at = format_timestamp();

    for package_id in &journal.desired_active_module_ids {
        let entry = lockfile.packages.get_mut(package_id).ok_or_else(|| {
            anyhow!("Cannot recover module '{package_id}': package is missing from lockfile")
        })?;
        if entry.package_type != "module" {
            bail!(
                "Cannot recover module '{}': lockfile type is '{}'",
                package_id,
                entry.package_type
            );
        }

        let entry_path = match &entry.module_activation {
            Some(existing) => existing.entry_path.clone(),
            None => reconstruct_entry_path(root, package_id, entry)?,
        };
        let import_mode = entry
            .module_import_mode
            .clone()
            .unwrap_or(ModuleImportMode::Module);
        entry.module_activation = Some(ModuleActivation {
            entry_path,
            import_mode,
            vendor_autoload_dir: journal.vendor_autoload_dir.clone(),
            managed_file_path: journal.managed_file_path.clone(),
            nu_executable_sha256: nu_paths.nu_executable_hash.clone(),
            nu_version: nu_paths.nu_version.clone(),
            activated_at: activated_at.clone(),
        });
    }

    for package_id in &journal.previous_active_module_ids {
        if desired.contains(package_id.as_str()) {
            continue;
        }
        let Some(entry) = lockfile.packages.get_mut(package_id) else {
            continue;
        };
        let should_clear = entry
            .module_activation
            .as_ref()
            .is_some_and(|activation| activation_matches_journal(activation, journal));
        if should_clear {
            entry.module_activation = None;
        }
    }

    lockfile.save(root)?;

    if journal.desired_file_exists {
        let managed_path = Path::new(&journal.managed_file_path);
        let state = AutoloadState::new(
            journal.vendor_autoload_dir.clone(),
            journal.managed_file_path.clone(),
            nu_paths.nu_executable_hash.clone(),
            nu_paths.nu_version.clone(),
            sha256_file(managed_path)?,
            journal.desired_active_module_ids.clone(),
            format_timestamp(),
        );
        state.save(root)?;
    } else {
        AutoloadState::delete(root)?;
    }

    Ok(())
}

fn reconstruct_entry_path(
    root: &Path,
    package_id: &str,
    entry: &LockfileEntry,
) -> Result<String> {
    let relative_entry = entry.entry.as_deref().ok_or_else(|| {
        anyhow!("Cannot recover module '{package_id}': entry is not set in lockfile")
    })?;
    if entry.payload_path.is_empty() {
        bail!("Cannot recover module '{package_id}': payload_path is not set in lockfile");
    }
    root.join(&entry.payload_path)
        .join(relative_entry)
        .to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("Cannot recover module '{package_id}': entry path is not valid UTF-8"))
}

fn activation_matches_journal(
    activation: &ModuleActivation,
    journal: &PendingAutoload,
) -> bool {
    activation.nu_executable_sha256 == journal.nu_executable_sha256
        && activation.nu_version == journal.nu_version
        && activation.vendor_autoload_dir == journal.vendor_autoload_dir
        && activation.managed_file_path == journal.managed_file_path
}
```

- [ ] **Step 5: Replace activation-side recovery and move planning under the lock**

In `execute_with_registrar_and_runner`:

1. Keep `--list` and `--check` read-only and before mutation locking.
2. Run stale-identity preflight.
3. Acquire a planning mutation lock.
4. Load the lockfile, call the existing plugin reconciler and new autoload reconciler, reload, and resolve targets while the lock is held.
5. Return `Nothing to activate` while still holding that planning lock when both target sets are empty.
6. Compute consent data, drop the planning lock, and prompt.
7. At the existing mutation checkpoint, reconcile both journals again, reload, and re-resolve before snapshot creation.

Use this shape for the planning checkpoint:

```rust
let planning_lock = acquire_mutation_lock(root)?;
let mut lockfile = Lockfile::load(root)?;
reconcile_plugin_journal(root, &nu_paths, &mut lockfile)?;
let recovery = reconcile_pending_autoload(root, &nu_paths, &mut lockfile)?;
print_autoload_recovery(recovery);
lockfile = Lockfile::load(root)?;

let plugin_targets = resolve_plugin_targets(args, &lockfile, &nu_paths, root)?;
let module_targets = resolve_module_targets(args, &lockfile, &nu_paths)?;
if plugin_targets.is_empty() && module_targets.is_empty() {
    println!("Nothing to activate.");
    return Ok(());
}
let managed_file_path = if module_targets.is_empty() {
    None
} else {
    Some(resolve_managed_file_path(&nu_paths)?)
};
drop(planning_lock);
```

Add this small UI-only helper; it must not perform state mutation:

```rust
fn print_autoload_recovery(outcome: AutoloadRecoveryOutcome) {
    match outcome {
        AutoloadRecoveryOutcome::NoJournal => {}
        AutoloadRecoveryOutcome::PreparedCleared => {
            eprintln!("   Module journal cleared (no external change occurred).");
        }
        AutoloadRecoveryOutcome::ReplacedCompleted => {
            eprintln!("   Module journal recovery complete.");
        }
    }
}
```

Delete the private `activate::reconcile_autoload_journal` function and remove its `AutoloadStage`, `RecoveryAction`, `ModuleActivation`, `AutoloadState`, and `sha256_file` imports when no longer used elsewhere.

- [ ] **Step 6: Replace deactivation-side recovery and move classification under the lock**

In `execute_with_runner`:

1. Load cached `NuPaths`.
2. Acquire a planning mutation lock.
3. Load the lockfile, call `reconcile_pending_autoload`, reload, and call `classify_and_validate_packages` while the lock is held.
4. Return `Nothing to deactivate` while still holding the planning lock when empty.
5. Compute stale/full-deactivation rules and consent data, drop the planning lock, and prompt.
6. Reacquire the mutation lock, reconcile again, reload, and reclassify targets before snapshot creation.

Use this shape before displaying the deactivation consent table:

```rust
let planning_lock = acquire_mutation_lock(root)?;
let mut lockfile = Lockfile::load(root)?;
let recovery = reconcile_pending_autoload(root, &nu_paths, &mut lockfile)?;
print_autoload_recovery(recovery);
lockfile = Lockfile::load(root)?;

let targets_requested = classify_and_validate_packages(args, &lockfile)?;
if targets_requested.is_empty() {
    println!("Nothing to deactivate.");
    return Ok(());
}

let total_with_any_activation = lockfile
    .packages
    .values()
    .filter(|pkg| pkg.package_type == "module" && pkg.module_activation.is_some())
    .count();
let is_full_deactivation = targets_requested.len() == total_with_any_activation;
let nu_is_stale = nu_paths.validate_drift().is_err();
if nu_is_stale && !is_full_deactivation {
    nu_paths.validate_drift()?;
}
drop(planning_lock);
```

After consent, shadow `targets_requested` with a freshly classified vector after the second recovery and lockfile reload. Recompute `is_full_deactivation` and drift checks from that locked snapshot before creating the new snapshot.

Delete the private `deactivate::reconcile_autoload_journal` function and remove recovery-only imports.

- [ ] **Step 7: Run focused tests and verify GREEN**

Run:

```bash
cargo test --test module_autoload_test recovery -- --nocapture
cargo test state::autoload_journal
cargo test cmd::activate
cargo test cmd::deactivate
```

Expected: PASS. The two Task 1 regressions pass, direct recovery tests pass, and existing activation/deactivation tests remain green.

- [ ] **Step 8: Format, inspect, and commit the working recovery refactor**

Run:

```bash
cargo fmt
git diff --check
git status --short
```

Then commit:

```bash
git add src/state/autoload_recovery.rs src/state/mod.rs src/cmd/activate.rs src/cmd/deactivate.rs tests/module_autoload_test.rs
git commit -m "Fix command-independent autoload recovery"
```

---

### Task 3: Update repository structure documentation and run all gates

**Files:**
- Modify: `AGENTS.md` project-structure section only if `src/state/autoload_recovery.rs` is not already represented by a broader entry.
- Verify: all files changed by Tasks 1-2.

**Interfaces:**
- Consumes: the final shared recovery API and command integration.
- Produces: repository documentation aligned with the new module and a fully verified branch.

- [ ] **Step 1: Document the new module**

Add this entry under `src/state/` in `AGENTS.md`:

```text
    autoload_recovery.rs — command-independent PendingAutoload reconciliation into lockfile + derived autoload state
```

Do not change unrelated phase status or command documentation.

- [ ] **Step 2: Run formatting and static gates**

Run:

```bash
cargo fmt --check
cargo clippy -- -D warnings
git diff --check
```

Expected: all commands exit 0. A Windows incremental-compilation cleanup warning may be reported, but no Clippy diagnostic may remain.

- [ ] **Step 3: Run the full test suite**

Run:

```bash
cargo test
```

Expected: all non-ignored tests pass. Do not run or claim the ignored real-Nu acceptance suite unless `nu --version` first confirms Nushell is present.

- [ ] **Step 4: Review the final diff against the spec**

Run:

```bash
git diff origin/master...HEAD --stat
git diff origin/master...HEAD -- src/state/autoload_recovery.rs src/cmd/activate.rs src/cmd/deactivate.rs tests/module_autoload_test.rs AGENTS.md
git status --short --branch
```

Confirm:

- one shared autoload reconciler remains;
- command planning occurs under the mutation lock before no-op exits;
- no prompt occurs while the lock is held;
- journal deletion is last;
- no schema, CLI, dependency, or unrelated documentation change appears.

- [ ] **Step 5: Commit documentation if it changed**

If `AGENTS.md` changed, run:

```bash
git add AGENTS.md
git commit -m "Document shared autoload recovery"
```

If it did not change, do not create an empty commit.
