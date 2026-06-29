use anyhow::{bail, Result};
use clap::Parser;
use std::path::PathBuf;

use crate::core::nu_version::NuVersion;
use crate::core::platform::Platform;
use crate::core::registry::RegistryManager;
use crate::core::resolve::Resolver;
use crate::install::transaction;
use crate::state::lifecycle_journal::{
    check_stale_journal, LifecycleOp, LifecycleStage, PendingLifecycle,
};
use crate::state::lockfile::Lockfile;

/// Update installed packages to their latest compatible versions
#[derive(Parser)]
pub struct UpdateArgs {
    /// Package to update (owner/name). Omit to check/update all packages.
    package: Option<String>,

    /// Report available updates without installing
    #[arg(long)]
    check: bool,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,
}

pub fn execute(args: &UpdateArgs, root: &PathBuf) -> Result<()> {
    if let Some(journal) = check_stale_journal(root)? {
        let op = match journal.op {
            LifecycleOp::Update => "update",
            LifecycleOp::Remove => "remove",
        };
        eprintln!(
            "Warning: A previous '{}' operation on '{}' was interrupted.",
            op, journal.package_id
        );
        eprintln!("Run `numan gc` to clean up any orphaned packages.");
    }

    let platform = Platform::detect();
    let nu_version = NuVersion::detect().unwrap_or_else(|e| {
        eprintln!("Warning: Could not detect Nu version: {e}");
        NuVersion {
            version: "unknown".to_string(),
            major: 0,
            minor: 0,
            patch: 0,
        }
    });

    let lockfile = Lockfile::load(root)?;
    if lockfile.is_empty() {
        println!("No packages installed.");
        return Ok(());
    }

    let registry = RegistryManager::new(root)?;
    let default_reg = registry.default_registry_name();

    let index = if registry.sig_path(&default_reg).exists() {
        registry.verify_and_load(&default_reg)?
    } else {
        if std::env::var("NUMAN_ALLOW_UNSIGNED").unwrap_or_default() != "1" {
            bail!(
                "Registry '{}' has no signature file. Set NUMAN_ALLOW_UNSIGNED=1 to override.",
                default_reg
            );
        }
        registry.load_index(&default_reg)?
    };

    let resolver = Resolver::new(&platform, &nu_version);

    let packages_to_check: Vec<String> = if let Some(ref id) = args.package {
        if !lockfile.packages.contains_key(id.as_str()) {
            bail!("Package '{}' is not installed.", id);
        }
        vec![id.clone()]
    } else {
        lockfile.packages.keys().cloned().collect()
    };

    // Phase 1: discover available updates
    let mut updates: Vec<(String, String, String, String)> = Vec::new(); // (id, from, to, orphan_path)

    for pkg_id in &packages_to_check {
        let current = match lockfile.packages.get(pkg_id.as_str()) {
            Some(e) => e,
            None => continue,
        };

        let pkg = match index.packages.iter().find(|p| p.id.to_string() == *pkg_id) {
            Some(p) => p,
            None => {
                if args.verbose {
                    println!("  {} not found in registry (skipping)", pkg_id);
                }
                continue;
            }
        };

        let resolved = match resolver.resolve(pkg) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let latest = resolved.version.to_string();
        if latest != current.version {
            updates.push((
                pkg_id.clone(),
                current.version.clone(),
                latest,
                current.payload_path().to_string(),
            ));
        }
    }

    if updates.is_empty() {
        println!("All packages are up to date.");
        return Ok(());
    }

    if args.check {
        println!("Updates available ({}):", updates.len());
        for (id, from, to, _) in &updates {
            println!("  {}  {} → {}", id, from, to);
        }
        return Ok(());
    }

    // Phase 2: apply updates
    println!("Updating {} package(s)...", updates.len());

    let options = transaction::InstallOptions {
        root,
        platform: &platform,
        nu_version: &nu_version,
        force: false,
        verbose: args.verbose,
    };

    for (pkg_id, from_version, to_version, orphan_path) in &updates {
        let journal = PendingLifecycle {
            op: LifecycleOp::Update,
            package_id: pkg_id.clone(),
            stage: LifecycleStage::Prepared,
            orphan_payload_path: Some(orphan_path.clone()),
            from_version: Some(from_version.clone()),
            to_version: Some(to_version.clone()),
        };
        journal.save(root)?;

        match transaction::install_package(pkg_id, Some(to_version), &options) {
            Ok(_) => {
                PendingLifecycle::clear(root)?;
                println!(
                    "{} {}  {} → {}",
                    console::style("✓").green(),
                    pkg_id,
                    from_version,
                    to_version
                );
            }
            Err(e) => {
                eprintln!("Failed to update {}: {}", pkg_id, e);
                eprintln!("Run `numan gc` to clean up orphaned packages.");
            }
        }
    }

    Ok(())
}
