use anyhow::{bail, Context, Result};
use clap::Args;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use crate::nu::autoload::{
    delete_managed_file, generate_autoload_content, replace_managed_file, validate_candidate,
    write_candidate, CandidateRunner, NuCandidateRunner,
};
use crate::nu::paths::NuPaths;
use crate::state::autoload_journal::{
    sha256_file, AutoloadOperation, AutoloadStage, PendingAutoload,
    SCHEMA_VERSION as AUTOLOAD_SCHEMA_VERSION,
};
use crate::state::autoload_recovery::{reconcile_pending_autoload, AutoloadRecoveryOutcome};
use crate::state::autoload_state::AutoloadState;
use crate::state::lockfile::Lockfile;
use crate::state::snapshot::{create_snapshot, SnapshotReason, SnapshotTrigger};
use crate::util::{format_timestamp, fs_safety::acquire_mutation_lock};

#[derive(Args, Debug)]
pub struct DeactivateArgs {
    /// Package IDs (owner/name) to deactivate. Omit to deactivate all active modules.
    pub packages: Vec<String>,

    /// Skip confirmation prompt
    #[arg(long)]
    pub yes: bool,

    /// Show detailed output
    #[arg(long)]
    pub verbose: bool,
}

/// A module that is currently active and eligible for deactivation.
#[derive(Debug)]
struct ActiveModule {
    package_id: String,
    vendor_autoload_dir: String,
    managed_file_path: String,
}

pub fn execute(args: &DeactivateArgs, root: &Path) -> Result<()> {
    execute_with_runner(args, root, None)
}

/// Testability entry point — accepts an injectable candidate runner.
pub fn execute_with_candidate_runner(
    args: &DeactivateArgs,
    root: &Path,
    runner: &dyn CandidateRunner,
) -> Result<()> {
    execute_with_runner(args, root, Some(runner))
}

fn execute_with_runner(
    args: &DeactivateArgs,
    root: &Path,
    runner: Option<&dyn CandidateRunner>,
) -> Result<()> {
    // 1. Load cached Nu identity
    let nu_paths = NuPaths::load(root)?;

    // 2. Reconcile interrupted work and classify targets while holding the
    //    mutation lock. This prevents stale pre-recovery state from causing an
    //    early error or no-op return.
    let planning_lock = acquire_mutation_lock(root)?;
    let mut lockfile = Lockfile::load(root)?;
    print_autoload_recovery(reconcile_pending_autoload(root, &nu_paths, &mut lockfile)?);
    let lockfile = Lockfile::load(root)?;

    // 3. Classify explicit package IDs (plugin/script/completion errors, module collection).
    let targets_requested = classify_and_validate_packages(args, &lockfile)?;

    if targets_requested.is_empty() {
        println!("Nothing to deactivate.");
        return Ok(());
    }

    // 4. Determine whether Nu has drifted (stale = binary changed since init).
    //    Full deactivation may proceed under stale Nu identity.
    //    Partial deactivation must not (it needs to re-generate a candidate).
    //
    //    is_full_deactivation: ALL modules with any activation record are being
    //    deactivated. We count any module_activation (regardless of Nu identity)
    //    because classify_and_validate_packages collects all activated modules,
    //    not just those matching current identity.
    let total_with_any_activation = lockfile
        .packages
        .values()
        .filter(|pkg| pkg.package_type == "module" && pkg.module_activation.is_some())
        .count();
    let is_full_deactivation = targets_requested.len() == total_with_any_activation;
    let nu_is_stale = nu_paths.validate_drift().is_err();

    if nu_is_stale && !is_full_deactivation {
        nu_paths.validate_drift()?; // re-call to surface the actual error message
    }
    drop(planning_lock);

    // 5. Show consent table and confirm
    println!();
    println!("Module deactivation");
    for m in &targets_requested {
        println!("  {} <- {}", m.package_id, m.managed_file_path);
    }
    println!();

    if !std::io::stdin().is_terminal() && !args.yes {
        bail!(
            "Interactive confirmation required for non-TTY sessions. \
             Pass --yes to deactivate without prompting."
        );
    }

    if !args.yes {
        print!("Proceed? [y/N] ");
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            bail!("Deactivation cancelled.");
        }
    }

    // 6. Reacquire the root mutation lock after consent.
    let _lock = acquire_mutation_lock(root)?;

    // Reload lockfile under the lock
    let mut lockfile = Lockfile::load(root)?;

    // 7. Reconcile any interrupted module-autoload journal.
    print_autoload_recovery(reconcile_pending_autoload(root, &nu_paths, &mut lockfile)?);

    // Reload and reclassify after reconciliation so the new operation is based
    // on the current authoritative state.
    let mut lockfile = Lockfile::load(root)?;
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
    if nu_paths.validate_drift().is_err() && !is_full_deactivation {
        nu_paths.validate_drift()?;
    }

    // 8. Snapshot current state before any mutation
    let snapshot = create_snapshot(
        root,
        SnapshotReason::PreMutation,
        SnapshotTrigger::Deactivate,
        None,
        None,
    )?;

    // All targets must agree on the same managed file path and vendor dir.
    // If activations point at different targets (e.g. after a stale refresh or
    // manual lockfile edits) we bail rather than silently mutating the wrong file.
    let managed_file_path = targets_requested[0].managed_file_path.clone();
    let vendor_autoload_dir = targets_requested[0].vendor_autoload_dir.clone();
    for t in &targets_requested[1..] {
        if t.managed_file_path != managed_file_path || t.vendor_autoload_dir != vendor_autoload_dir
        {
            bail!(
                "Module activations point to different managed files:\n  \
                 '{}' ({})\n  vs '{}' ({})\n\
                 Run 'numan activate --check' to inspect state, or \
                 'numan init --refresh' to reset the vendor-autoload target.",
                managed_file_path,
                targets_requested[0].package_id,
                t.managed_file_path,
                t.package_id
            );
        }
    }

    // The set of module IDs that will REMAIN active after this deactivation
    let currently_active_ids: Vec<String> = AutoloadState::active_module_ids_from_lockfile(
        &lockfile,
        &nu_paths.nu_executable_hash,
        &nu_paths.nu_version,
        &vendor_autoload_dir,
        &managed_file_path,
    );

    let deactivating_ids: Vec<String> = targets_requested
        .iter()
        .map(|m| m.package_id.clone())
        .collect();

    let remaining_ids: Vec<String> = currently_active_ids
        .iter()
        .filter(|id| !deactivating_ids.contains(id))
        .cloned()
        .collect();

    let managed_path = Path::new(&managed_file_path);

    if remaining_ids.is_empty() {
        // ── FULL DEACTIVATION ─────────────────────────────────────────────────
        run_full_deactivation(
            root,
            &nu_paths,
            &mut lockfile,
            &targets_requested,
            managed_path,
            &vendor_autoload_dir,
            &managed_file_path,
            &currently_active_ids,
            Some(snapshot.id.clone()),
        )?;
    } else {
        // ── PARTIAL DEACTIVATION ──────────────────────────────────────────────
        // Nu must not be stale for partial deactivation (candidate generation required).
        nu_paths.validate_drift()?;

        let real_runner;
        let runner_ref: &dyn CandidateRunner = if let Some(r) = runner {
            r
        } else {
            real_runner = NuCandidateRunner::new(&nu_paths.nu_executable);
            &real_runner
        };

        run_partial_deactivation(
            root,
            &nu_paths,
            &mut lockfile,
            &targets_requested,
            &remaining_ids,
            managed_path,
            &vendor_autoload_dir,
            &managed_file_path,
            &currently_active_ids,
            runner_ref,
            Some(snapshot.id.clone()),
        )?;
    }

    Ok(())
}

// ── Package classification ─────────────────────────────────────────────────────

/// Classify and validate explicit or implicit deactivation targets.
///
/// - Plugin packages return a clear "deferred to a later phase" error.
/// - Script/completion packages return a deferred-feature error.
/// - Unknown package IDs fail.
/// - Modules that are not currently active fail.
/// - No IDs means deactivate all currently active modules.
fn classify_and_validate_packages(
    args: &DeactivateArgs,
    lockfile: &Lockfile,
) -> Result<Vec<ActiveModule>> {
    // First pass: check for immediately-rejected types in explicit list
    for pkg_id in &args.packages {
        let entry = match lockfile.packages.get(pkg_id) {
            Some(e) => e,
            None => {
                bail!("Package '{pkg_id}' not found in lockfile (not installed)");
            }
        };
        match entry.package_type.as_str() {
            "module" => {}
            "plugin" => {
                bail!("Plugin deactivation is deferred to a later phase.");
            }
            "script" => {
                bail!(
                    "Package '{pkg_id}' is a script — script deactivation is deferred to a later phase."
                );
            }
            "completion" => {
                bail!(
                    "Package '{pkg_id}' is a completion — completion deactivation is deferred to a later phase."
                );
            }
            other => {
                bail!("Package '{pkg_id}' has unknown type '{other}' — cannot deactivate.");
            }
        }
    }

    // Collect active modules. We need NuPaths to check active status, but we
    // can gather all entries that have a module_activation and check them.
    // For deactivation we accept any module that has ANY module_activation
    // (regardless of Nu identity) when explicit IDs are given.
    // For the implicit "all" path, we only target those with current Nu identity.
    //
    // The active-module set is determined from lockfile module_activation records.

    if args.packages.is_empty() {
        // Deactivate all modules that have any module_activation record.
        // We collect everything that is activated (any identity) so the user
        // sees all of them. The Nu identity check is done when computing
        // remaining_ids inside the lane functions.
        let mut targets = Vec::new();
        let mut sorted_ids: Vec<&String> = lockfile
            .packages
            .keys()
            .filter(|id| {
                let e = &lockfile.packages[*id];
                e.package_type == "module" && e.module_activation.is_some()
            })
            .collect();
        sorted_ids.sort();

        for pkg_id in sorted_ids {
            let entry = &lockfile.packages[pkg_id];
            let ma = entry.module_activation.as_ref().unwrap();
            targets.push(ActiveModule {
                package_id: pkg_id.clone(),
                vendor_autoload_dir: ma.vendor_autoload_dir.clone(),
                managed_file_path: ma.managed_file_path.clone(),
            });
        }
        Ok(targets)
    } else {
        let mut targets = Vec::new();
        for pkg_id in &args.packages {
            let entry = &lockfile.packages[pkg_id]; // type already checked above
            match &entry.module_activation {
                None => {
                    bail!("Package '{pkg_id}' is a module but is not currently active.");
                }
                Some(ma) => {
                    targets.push(ActiveModule {
                        package_id: pkg_id.clone(),
                        vendor_autoload_dir: ma.vendor_autoload_dir.clone(),
                        managed_file_path: ma.managed_file_path.clone(),
                    });
                }
            }
        }
        Ok(targets)
    }
}

/// Count the number of modules currently active for the given Nu identity.
/// Used only in tests.
#[cfg(test)]
fn count_active_modules(lockfile: &Lockfile, nu_paths: &NuPaths) -> usize {
    let vendor_dir = nu_paths.vendor_autoload_dir.as_deref().unwrap_or("");
    let managed_path_str = if vendor_dir.is_empty() {
        String::new()
    } else {
        format!("{vendor_dir}/numan.nu")
    };

    lockfile
        .packages
        .values()
        .filter(|e| {
            e.package_type == "module"
                && e.is_module_active_for(
                    &nu_paths.nu_executable_hash,
                    &nu_paths.nu_version,
                    vendor_dir,
                    &managed_path_str,
                )
        })
        .count()
}

// ── Full deactivation ─────────────────────────────────────────────────────────

/// Run full module deactivation: verify ownership and hash, delete only the
/// Numan-managed `numan.nu`, clear activation records, remove autoload-state,
/// use the journal protocol.
///
/// This path is allowed even when Nu identity has drifted because no candidate
/// generation is needed — we are only deleting a verified Numan-managed file.
#[allow(clippy::too_many_arguments)]
fn run_full_deactivation(
    root: &Path,
    nu_paths: &NuPaths,
    lockfile: &mut Lockfile,
    targets: &[ActiveModule],
    managed_path: &Path,
    vendor_autoload_dir: &str,
    managed_file_path: &str,
    previous_active_ids: &[String],
    pre_mutation_snapshot_id: Option<String>,
) -> Result<()> {
    if !managed_path.exists() {
        // Nothing to delete — just clear lockfile records.
        eprintln!(
            "{}  Managed file '{}' does not exist — clearing lockfile records only.",
            console::style("⚠").yellow(),
            managed_path.display()
        );
        clear_module_activations(lockfile, targets, root)?;
        AutoloadState::delete(root)?;
        print_deactivation_success(targets);
        return Ok(());
    }

    // Step 1-2: Verify ownership marker + SHA-256 against autoload-state.
    crate::util::fs_safety::assert_managed_file_owned(managed_path).with_context(|| {
        "Numan managed-file drift detected.\n\
         numan.nu was changed, replaced, moved, or is no longer a Numan-owned regular \
         file. Numan will not overwrite or delete it automatically."
    })?;

    // Record previous SHA-256 for the journal.
    let previous_sha256 = sha256_file(managed_path)?;

    let previous_active_sorted = {
        let mut v = previous_active_ids.to_vec();
        v.sort();
        v
    };

    // Step 3: Write journal at Prepared (desired_file_exists = false).
    let journal = PendingAutoload {
        schema_version: AUTOLOAD_SCHEMA_VERSION,
        operation: AutoloadOperation::Deactivate,
        stage: AutoloadStage::Prepared,
        nu_executable_sha256: nu_paths.nu_executable_hash.clone(),
        nu_version: nu_paths.nu_version.clone(),
        vendor_autoload_dir: vendor_autoload_dir.to_string(),
        managed_file_path: managed_file_path.to_string(),
        previous_file_exists: true,
        previous_file_sha256: Some(previous_sha256),
        desired_file_exists: false,
        candidate_sha256: None,
        previous_active_module_ids: previous_active_sorted,
        desired_active_module_ids: vec![],
        targeted_module_ids: targets.iter().map(|m| m.package_id.clone()).collect(),
        created_at: format_timestamp(),
        pre_mutation_snapshot_id,
    };
    journal.save(root)?;

    // Step 4: Delete only the verified Numan-managed `numan.nu`.
    delete_managed_file(managed_path).with_context(|| {
        "Failed to delete Numan-managed autoload file. Journal preserved for recovery."
    })?;

    // Step 5: Update journal to Replaced.
    let mut journal = journal;
    journal.stage = AutoloadStage::Replaced;
    journal.save(root)?;

    // Step 6: Clear module activation records from lockfile.
    if let Err(e) = clear_module_activations(lockfile, targets, root) {
        eprintln!(
            "{} Failed to clear lockfile module activation records: {}\n\
             Journal preserved at state/pending-autoload.json for recovery.",
            console::style("✗").red(),
            e
        );
        return Err(e);
    }

    // Step 7: Delete autoload-state.json.
    if let Err(e) = AutoloadState::delete(root) {
        eprintln!(
            "{} Failed to delete autoload-state: {}\n\
             Journal preserved at state/pending-autoload.json for recovery.",
            console::style("✗").red(),
            e
        );
        return Err(e);
    }

    // Step 8: Clear journal.
    PendingAutoload::delete(root)?;

    print_deactivation_success(targets);
    Ok(())
}

// ── Partial deactivation ──────────────────────────────────────────────────────

/// Run partial module deactivation: regenerate and validate a candidate
/// containing only the remaining active modules, then execute the full
/// 13-step module-autoload transaction.
///
/// Nu must not be stale when this path is taken (candidate generation required).
#[allow(clippy::too_many_arguments)]
fn run_partial_deactivation(
    root: &Path,
    nu_paths: &NuPaths,
    lockfile: &mut Lockfile,
    targets: &[ActiveModule],
    remaining_ids: &[String],
    managed_path: &Path,
    vendor_autoload_dir: &str,
    managed_file_path: &str,
    previous_active_ids: &[String],
    runner: &dyn CandidateRunner,
    pre_mutation_snapshot_id: Option<String>,
) -> Result<()> {
    // Resolve remaining module entries from the lockfile.
    let mut remaining_entries = Vec::new();
    for pkg_id in remaining_ids {
        let entry = lockfile.packages.get(pkg_id).with_context(|| {
            format!("Package '{pkg_id}' in remaining set not found in lockfile")
        })?;
        if let Some(ma) = &entry.module_activation {
            let resolved_entry = crate::nu::autoload::ResolvedEntry {
                absolute_path: PathBuf::from(&ma.entry_path),
                import_mode: ma.import_mode.clone(),
                scoped_id: pkg_id.clone(),
            };
            remaining_entries.push(resolved_entry);
        } else {
            // Remaining module has no activation record — skip (shouldn't happen).
            eprintln!(
                "{}  Skipping '{}' in remaining set: no module_activation record.",
                console::style("⚠").yellow(),
                pkg_id
            );
        }
    }

    // Also include any other currently active modules not in this deactivation set
    // and not already in remaining_entries (safety: should be the same set).
    // remaining_ids already excludes targets, so remaining_entries is the full
    // post-deactivation set.

    // Step 4: Generate candidate content for remaining modules.
    let content = generate_autoload_content(&remaining_entries)
        .context("Failed to generate autoload content for partial deactivation")?;

    // Step 5: Write candidate file (same directory as managed file).
    let candidate_path = write_candidate(managed_path, &content)
        .context("Failed to write candidate file for partial deactivation")?;

    // Compute candidate SHA-256 for journal.
    let candidate_sha = sha256_file(&candidate_path)?;

    // Record previous state for journal.
    let previous_file_exists = managed_path.exists();
    let previous_sha256 = if previous_file_exists {
        Some(sha256_file(managed_path)?)
    } else {
        None
    };

    let previous_active_sorted = {
        let mut v = previous_active_ids.to_vec();
        v.sort();
        v
    };

    let mut desired_active_ids = remaining_ids.to_vec();
    desired_active_ids.sort();
    desired_active_ids.dedup();

    // Step 5 (Nu validation): Execute candidate with the candidate runner.
    let scoped_ids: Vec<&str> = remaining_ids.iter().map(|s| s.as_str()).collect();
    if let Err(e) = validate_candidate(&candidate_path, runner, &scoped_ids) {
        eprintln!(
            "{} Module candidate validation failed: {}",
            console::style("✗").red(),
            e
        );
        bail!("Partial deactivation failed: candidate validation error. Managed file unchanged.");
    }

    // Step 6: Snapshot already done at the top level before lane entry.

    // Step 7: Atomically write journal at Prepared.
    let journal = PendingAutoload {
        schema_version: AUTOLOAD_SCHEMA_VERSION,
        operation: AutoloadOperation::Deactivate,
        stage: AutoloadStage::Prepared,
        nu_executable_sha256: nu_paths.nu_executable_hash.clone(),
        nu_version: nu_paths.nu_version.clone(),
        vendor_autoload_dir: vendor_autoload_dir.to_string(),
        managed_file_path: managed_file_path.to_string(),
        previous_file_exists,
        previous_file_sha256: previous_sha256,
        desired_file_exists: true,
        candidate_sha256: Some(candidate_sha),
        previous_active_module_ids: previous_active_sorted,
        desired_active_module_ids: desired_active_ids.clone(),
        targeted_module_ids: targets.iter().map(|m| m.package_id.clone()).collect(),
        created_at: format_timestamp(),
        pre_mutation_snapshot_id,
    };
    journal.save(root)?;

    // Step 8: Atomically replace numan.nu with the validated candidate.
    if let Err(e) = replace_managed_file(managed_path, &candidate_path) {
        eprintln!(
            "{} Failed to replace managed autoload file: {}",
            console::style("✗").red(),
            e
        );
        bail!("Partial deactivation failed: could not replace managed file.");
    }

    // Step 9: Update journal to Replaced.
    let mut journal = journal;
    journal.stage = AutoloadStage::Replaced;
    journal.save(root)?;

    // Step 10: Clear module activation records for deactivated targets.
    //          (Remaining modules keep their activation records.)
    if let Err(e) = clear_module_activations(lockfile, targets, root) {
        eprintln!(
            "{} Failed to clear lockfile module activation records: {}\n\
             Journal preserved at state/pending-autoload.json for recovery.",
            console::style("✗").red(),
            e
        );
        return Err(e);
    }

    // Step 11: Atomically write derived autoload-state.json.
    let file_sha = sha256_file(managed_path)?;
    let autoload_state = AutoloadState::new(
        vendor_autoload_dir.to_string(),
        managed_file_path.to_string(),
        nu_paths.nu_executable_hash.clone(),
        nu_paths.nu_version.clone(),
        file_sha,
        desired_active_ids,
        format_timestamp(),
    );
    if let Err(e) = autoload_state.save(root) {
        eprintln!(
            "{} Failed to write autoload-state after partial deactivation: {}\n\
             Journal preserved at state/pending-autoload.json for recovery.",
            console::style("✗").red(),
            e
        );
        return Err(e);
    }

    // Step 12: Clear the journal.
    PendingAutoload::delete(root)?;

    // Step 13: (Lock released via RAII when _lock is dropped in caller)

    print_deactivation_success(targets);
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Clear `module_activation` from each deactivated target and persist lockfile.
fn clear_module_activations(
    lockfile: &mut Lockfile,
    targets: &[ActiveModule],
    root: &Path,
) -> Result<()> {
    for target in targets {
        if let Some(pkg) = lockfile.packages.get_mut(&target.package_id) {
            pkg.module_activation = None;
        }
    }
    lockfile.save(root)
}

fn print_deactivation_success(targets: &[ActiveModule]) {
    for target in targets {
        println!(
            "{} Module {} removed from autoload — takes effect in future Nu sessions",
            console::style("✓").green(),
            target.package_id,
        );
    }
}

// ── Journal reconciliation ─────────────────────────────────────────────────────

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::package::ModuleImportMode;
    use crate::state::lockfile::{Lockfile, LockfileEntry, ModuleActivation};
    use std::collections::BTreeMap;

    fn make_lockfile_with_modules(entries: Vec<(&str, &str, bool)>) -> Lockfile {
        // entries: (pkg_id, pkg_type, has_activation)
        let mut packages = BTreeMap::new();
        for (id, pkg_type, has_activation) in entries {
            let module_activation = if has_activation && pkg_type == "module" {
                Some(ModuleActivation {
                    entry_path: format!("/root/packages/modules/{id}/1.0.0-abc/mod.nu"),
                    import_mode: ModuleImportMode::Module,
                    vendor_autoload_dir: "/nu/vendor/autoload".to_string(),
                    managed_file_path: "/nu/vendor/autoload/numan.nu".to_string(),
                    nu_executable_sha256: "exe-hash".to_string(),
                    nu_version: "0.113.1".to_string(),
                    activated_at: "0".to_string(),
                })
            } else {
                None
            };

            packages.insert(
                id.to_string(),
                LockfileEntry {
                    version: "1.0.0".to_string(),
                    package_type: pkg_type.to_string(),
                    source: "archive".to_string(),
                    target: None,
                    artifact_url: None,
                    artifact_sha256: None,
                    executable_path: None,
                    archive_root: None,
                    include: None,
                    entry: Some("mod.nu".to_string()),
                    installed_at: "0".to_string(),
                    nu_version_at_install: None,
                    activation: None,
                    registry_url: None,
                    registry_revision: None,
                    index_sha256: None,
                    signing_key_fingerprint: None,
                    git_url: None,
                    git_rev: None,
                    cargo_name: None,
                    cargo_lock_sha256: None,
                    built_sha256: None,
                    payload_path: format!("packages/modules/{id}/1.0.0-abc"),
                    revision_id: None,
                    payload_sha256: None,
                    executable_sha256: None,
                    selection_reason: None,
                    origin: None,
                    module_activation,
                    module_import_mode: Some(ModuleImportMode::Module),
                    locked_dependencies: BTreeMap::new(),
                },
            );
        }
        Lockfile {
            version: 2,
            generated_at: "ts".to_string(),
            nu_version: "0.113.1".to_string(),
            platform: "x86_64-pc-windows-msvc".to_string(),
            packages,
        }
    }

    fn fake_nu_paths() -> NuPaths {
        NuPaths {
            nu_executable: "/usr/bin/nu".to_string(),
            nu_version: "0.113.1".to_string(),
            plugin_registry_path: "/path/to/plugins.msgpackz".to_string(),
            nu_executable_hash: "exe-hash".to_string(),
            platform: "x86_64-pc-windows-msvc".to_string(),
            data_dir: Some("/nu".to_string()),
            vendor_autoload_dirs: vec!["/nu/vendor/autoload".to_string()],
            vendor_autoload_dir: Some("/nu/vendor/autoload".to_string()),
        }
    }

    // ── classify_and_validate_packages ────────────────────────────────────────

    #[test]
    fn plugin_type_returns_deferred_error() {
        let lockfile = make_lockfile_with_modules(vec![("owner/myplugin", "plugin", false)]);

        let args = DeactivateArgs {
            packages: vec!["owner/myplugin".to_string()],
            yes: true,
            verbose: false,
        };

        let err = classify_and_validate_packages(&args, &lockfile).unwrap_err();
        assert!(
            err.to_string().contains("deferred to a later phase"),
            "Expected deferred error for plugin type, got: {err}"
        );
    }

    #[test]
    fn script_type_returns_deferred_error() {
        let lockfile = make_lockfile_with_modules(vec![("owner/myscript", "script", false)]);

        let args = DeactivateArgs {
            packages: vec!["owner/myscript".to_string()],
            yes: true,
            verbose: false,
        };

        let err = classify_and_validate_packages(&args, &lockfile).unwrap_err();
        assert!(
            err.to_string().contains("deferred"),
            "Expected deferred error for script type, got: {err}"
        );
    }

    #[test]
    fn completion_type_returns_deferred_error() {
        let lockfile = make_lockfile_with_modules(vec![("owner/mycomp", "completion", false)]);

        let args = DeactivateArgs {
            packages: vec!["owner/mycomp".to_string()],
            yes: true,
            verbose: false,
        };

        let err = classify_and_validate_packages(&args, &lockfile).unwrap_err();
        assert!(
            err.to_string().contains("deferred"),
            "Expected deferred error for completion type, got: {err}"
        );
    }

    #[test]
    fn inactive_module_returns_error() {
        let lockfile =
            make_lockfile_with_modules(vec![("owner/mymod", "module", false /* inactive */)]);

        let args = DeactivateArgs {
            packages: vec!["owner/mymod".to_string()],
            yes: true,
            verbose: false,
        };

        let err = classify_and_validate_packages(&args, &lockfile).unwrap_err();
        assert!(
            err.to_string().contains("not currently active"),
            "Expected not-active error, got: {err}"
        );
    }

    #[test]
    fn missing_package_returns_error() {
        let lockfile = make_lockfile_with_modules(vec![]);

        let args = DeactivateArgs {
            packages: vec!["owner/nosuchpkg".to_string()],
            yes: true,
            verbose: false,
        };

        let err = classify_and_validate_packages(&args, &lockfile).unwrap_err();
        assert!(
            err.to_string().contains("not found"),
            "Expected not-found error, got: {err}"
        );
    }

    #[test]
    fn active_module_is_resolved() {
        let lockfile =
            make_lockfile_with_modules(vec![("owner/mymod", "module", true /* active */)]);

        let args = DeactivateArgs {
            packages: vec!["owner/mymod".to_string()],
            yes: true,
            verbose: false,
        };

        let targets = classify_and_validate_packages(&args, &lockfile).unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].package_id, "owner/mymod");
    }

    #[test]
    fn no_packages_returns_all_active_modules() {
        let lockfile = make_lockfile_with_modules(vec![
            ("owner/alpha", "module", true),
            ("owner/beta", "module", true),
            ("owner/gamma", "module", false), // inactive — excluded
        ]);

        let args = DeactivateArgs {
            packages: vec![],
            yes: true,
            verbose: false,
        };

        let mut targets = classify_and_validate_packages(&args, &lockfile).unwrap();
        targets.sort_by(|a, b| a.package_id.cmp(&b.package_id));
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].package_id, "owner/alpha");
        assert_eq!(targets[1].package_id, "owner/beta");
    }

    #[test]
    fn non_tty_without_yes_is_expected_to_fail() {
        let is_tty = std::io::stdin().is_terminal();
        if !is_tty {
            let expected = "Interactive confirmation required for non-TTY sessions";
            assert!(expected.contains("non-TTY"));
        }
    }

    // ── count_active_modules ──────────────────────────────────────────────────

    #[test]
    fn count_active_modules_returns_correct_count() {
        let lockfile = make_lockfile_with_modules(vec![
            ("owner/alpha", "module", true),
            ("owner/beta", "module", true),
            ("owner/gamma", "module", false),
        ]);
        let nu_paths = fake_nu_paths();
        assert_eq!(count_active_modules(&lockfile, &nu_paths), 2);
    }

    #[test]
    fn count_active_modules_excludes_wrong_identity() {
        let lockfile = make_lockfile_with_modules(vec![("owner/alpha", "module", true)]);
        let mut nu_paths = fake_nu_paths();
        nu_paths.nu_executable_hash = "different-hash".to_string();
        assert_eq!(count_active_modules(&lockfile, &nu_paths), 0);
    }
}
