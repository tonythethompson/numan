use anyhow::{bail, Context, Result};
use clap::Parser;
use std::path::Path;

use crate::state::lifecycle_journal::{LifecycleOp, LifecycleStage, PendingLifecycle};
use crate::state::lockfile::Lockfile;
use crate::state::snapshot::{create_snapshot, SnapshotReason, SnapshotTrigger};
use crate::state::nupm_import::NupmImportsFile;
use crate::util::fs_safety::acquire_mutation_lock;

/// Remove an installed package
#[derive(Parser)]
pub struct RemoveArgs {
    /// Package to remove (owner/name)
    package: String,

    /// Remove even if package has an active activation record
    #[arg(long)]
    force: bool,
}

pub fn execute(args: &RemoveArgs, root: &Path) -> Result<()> {
    let _lock = acquire_mutation_lock(root)?;

    let mut lockfile = Lockfile::load(root)?;

    let entry = match lockfile.packages.get(&args.package) {
        Some(e) => e.clone(),
        None => bail!("Package '{}' is not installed.", args.package),
    };

    if !args.force {
        if entry.activation.is_some() {
            bail!(
                "Package '{}' is currently active as a plugin. \
                 Deactivate it first or use --force.",
                args.package
            );
        }
        if entry.module_activation.is_some() {
            bail!(
                "Package '{}' is currently active as a module. \
                 Run `numan deactivate {}` first or use --force.",
                args.package,
                args.package
            );
        }
    }

    let payload_path = entry.payload_path().to_string();
    let payload_dir = root.join(&payload_path);

    // Snapshot current state before any mutation or journal write so the
    // pre-remove activation graph is recoverable via `numan snapshot rollback`.
    create_snapshot(
        root,
        SnapshotReason::PreMutation,
        SnapshotTrigger::Remove,
        None,
        None,
    )?;

    // Write lifecycle journal before any mutation so a crash is detectable.
    let journal = PendingLifecycle {
        op: LifecycleOp::Remove,
        package_id: args.package.clone(),
        stage: LifecycleStage::Prepared,
        orphan_payload_path: Some(payload_path.clone()),
        from_version: None,
        to_version: None,
        nupm_source_path: None,
        nupm_metadata_sha256: None,
        staging_dir: None,
        promoted_payload_path: None,
        batch_package_ids: Vec::new(),
        batch_staging_dirs: Vec::new(),
        target_snapshot_id: None,
        pre_rollback_snapshot_id: None,
    };
    journal.save(root)?;

    // Remove from lockfile (atomic write).
    lockfile.packages.remove(&args.package);
    lockfile.save(root)?;

    let mut imports = NupmImportsFile::load(root)?;
    if imports.remove(&args.package) {
        imports.save(root)?;
    }

    // Advance journal so a crash here is recoverable: lockfile is already
    // updated; the payload dir is the only thing left to clean.
    PendingLifecycle {
        stage: LifecycleStage::LockfileUpdated,
        ..journal
    }
    .save(root)?;

    // Delete payload directory. A failure here is non-fatal: `numan gc` will
    // clean up the orphaned directory on the next run.
    if payload_dir.exists() {
        if let Err(e) = std::fs::remove_dir_all(&payload_dir)
            .with_context(|| format!("Failed to delete {}", payload_dir.display()))
        {
            eprintln!("Warning: {e}");
            eprintln!("Run `numan gc` to finish cleanup.");
        }
    }

    PendingLifecycle::clear(root)?;

    println!("{} Removed {}", console::style("✓").green(), args.package);

    Ok(())
}
