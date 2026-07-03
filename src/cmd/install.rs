use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

use crate::core::nu_version::NuVersion;
use crate::core::platform::Platform;
use crate::install::transaction;
use crate::util::fs_safety::acquire_mutation_lock;

/// Install a package
#[derive(Parser)]
pub struct InstallArgs {
    /// Package to install (owner/name or owner/name@version)
    package: String,

    /// Force reinstall even if already installed
    #[arg(long)]
    force: bool,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,
}

pub fn execute(args: &InstallArgs, root: &PathBuf) -> Result<()> {
    let _lock = acquire_mutation_lock(root)?;

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

    let options = transaction::InstallOptions {
        root,
        platform: &platform,
        nu_version: &nu_version,
        force: args.force,
        verbose: args.verbose,
        registry_name: None,
        snapshot_trigger: crate::state::snapshot::SnapshotTrigger::Install,
    };

    let version = if args.package.contains('@') {
        Some(args.package.split('@').nth(1).unwrap_or(""))
    } else {
        None
    };

    let package_id = args.package.split('@').next().unwrap_or(&args.package);

    transaction::install_package(package_id, version, &options)?;

    Ok(())
}
