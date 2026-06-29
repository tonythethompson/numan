use anyhow::{bail, Result};
use clap::Parser;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::core::nu_version::NuVersion;
use crate::core::package::RegistryIndex;
use crate::core::platform::Platform;
use crate::core::registry::RegistryManager;
use crate::core::resolve::Resolver;
use crate::install::transaction;
use crate::state::lifecycle_journal::{
    check_stale_journal, LifecycleOp, LifecycleStage, PendingLifecycle,
};
use crate::state::lockfile::{Lockfile, LockfileEntry};
use crate::util::fs_safety::acquire_mutation_lock;

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

struct PendingUpdate {
    package_id: String,
    from_version: String,
    to_version: String,
    orphan_path: String,
    registry_name: String,
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

    if args.check {
        let lockfile = Lockfile::load(root)?;
        if lockfile.is_empty() {
            println!("No packages installed.");
            return Ok(());
        }

        let registry = RegistryManager::new(root)?;
        let default_reg = registry.default_registry_name();
        let resolver = Resolver::new(&platform, &nu_version);
        let packages_to_check = packages_to_check(args, &lockfile)?;
        let mut index_cache = HashMap::new();
        let updates = discover_pending_updates(
            &lockfile,
            &packages_to_check,
            &default_reg,
            &registry,
            &resolver,
            args.verbose,
            &mut index_cache,
        )?;

        if updates.is_empty() {
            println!("All packages are up to date.");
            return Ok(());
        }

        println!("Updates available ({}):", updates.len());
        for update in &updates {
            println!(
                "  {}  {} → {}",
                update.package_id, update.from_version, update.to_version
            );
        }
        return Ok(());
    }

    let _lock = acquire_mutation_lock(root)?;

    let lockfile = Lockfile::load(root)?;
    if lockfile.is_empty() {
        println!("No packages installed.");
        return Ok(());
    }

    let registry = RegistryManager::new(root)?;
    let default_reg = registry.default_registry_name();
    let resolver = Resolver::new(&platform, &nu_version);
    let packages_to_check = packages_to_check(args, &lockfile)?;
    let mut index_cache = HashMap::new();
    let updates = discover_pending_updates(
        &lockfile,
        &packages_to_check,
        &default_reg,
        &registry,
        &resolver,
        args.verbose,
        &mut index_cache,
    )?;

    if updates.is_empty() {
        println!("All packages are up to date.");
        return Ok(());
    }

    println!("Updating {} package(s)...", updates.len());

    let mut failures: Vec<String> = Vec::new();

    for update in &updates {
        let current = match lockfile.packages.get(update.package_id.as_str()) {
            Some(entry) => entry,
            None => continue,
        };

        if let Err(e) = ensure_not_active(current, &update.package_id) {
            if args.package.is_some() {
                return Err(e);
            }
            eprintln!("  {e}");
            failures.push(update.package_id.clone());
            continue;
        }

        let journal = PendingLifecycle {
            op: LifecycleOp::Update,
            package_id: update.package_id.clone(),
            stage: LifecycleStage::Prepared,
            orphan_payload_path: Some(update.orphan_path.clone()),
            from_version: Some(update.from_version.clone()),
            to_version: Some(update.to_version.clone()),
        };
        journal.save(root)?;

        let options = transaction::InstallOptions {
            root,
            platform: &platform,
            nu_version: &nu_version,
            force: false,
            verbose: args.verbose,
            registry_name: Some(&update.registry_name),
        };

        match transaction::install_package(&update.package_id, Some(&update.to_version), &options) {
            Ok(_) => {
                PendingLifecycle {
                    stage: LifecycleStage::LockfileUpdated,
                    ..journal
                }
                .save(root)?;
                PendingLifecycle::clear(root)?;
                println!(
                    "{} {}  {} → {}",
                    console::style("✓").green(),
                    update.package_id,
                    update.from_version,
                    update.to_version
                );
            }
            Err(e) => {
                eprintln!("Failed to update {}: {}", update.package_id, e);
                eprintln!("Run `numan gc` to clean up orphaned packages.");
                failures.push(update.package_id.clone());
            }
        }
    }

    if !failures.is_empty() {
        bail!(
            "Failed to update {} package(s): {}",
            failures.len(),
            failures.join(", ")
        );
    }

    Ok(())
}

fn packages_to_check(args: &UpdateArgs, lockfile: &Lockfile) -> Result<Vec<String>> {
    if let Some(ref id) = args.package {
        if !lockfile.packages.contains_key(id.as_str()) {
            bail!("Package '{}' is not installed.", id);
        }
        Ok(vec![id.clone()])
    } else {
        Ok(lockfile.packages.keys().cloned().collect())
    }
}

fn discover_pending_updates(
    lockfile: &Lockfile,
    packages_to_check: &[String],
    default_reg: &str,
    registry: &RegistryManager,
    resolver: &Resolver,
    verbose: bool,
    index_cache: &mut HashMap<String, RegistryIndex>,
) -> Result<Vec<PendingUpdate>> {
    let mut updates = Vec::new();

    for pkg_id in packages_to_check {
        let current = match lockfile.packages.get(pkg_id.as_str()) {
            Some(e) => e,
            None => continue,
        };

        let registry_name = registry_name_for_entry(current, default_reg);
        let index = match index_cache.get(&registry_name) {
            Some(idx) => idx,
            None => {
                let loaded = registry.load_verified(&registry_name)?;
                index_cache.insert(registry_name.clone(), loaded.index);
                index_cache.get(&registry_name).expect("just inserted")
            }
        };

        let pkg = match index.packages.iter().find(|p| p.id.to_string() == *pkg_id) {
            Some(p) => p,
            None => {
                if verbose {
                    println!(
                        "  {} not found in registry '{}' (skipping)",
                        pkg_id, registry_name
                    );
                }
                continue;
            }
        };

        let resolved = match resolver.resolve(pkg) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let latest = resolved.version.to_string();
        if is_upgrade_available(&current.version, &resolved.version) {
            updates.push(PendingUpdate {
                package_id: pkg_id.clone(),
                from_version: current.version.clone(),
                to_version: latest,
                orphan_path: current.payload_path().to_string(),
                registry_name,
            });
        }
    }

    Ok(updates)
}

/// Parse `registry:official` from the lockfile entry's recorded origin.
fn registry_name_for_entry(entry: &LockfileEntry, default: &str) -> String {
    entry
        .origin
        .as_deref()
        .or(entry.registry_url.as_deref())
        .and_then(|value| value.strip_prefix("registry:"))
        .unwrap_or(default)
        .to_string()
}

/// Returns true when `resolved` is strictly newer than the installed version.
fn is_upgrade_available(current: &str, resolved: &semver::Version) -> bool {
    match current.parse::<semver::Version>() {
        Ok(current_ver) => resolved > &current_ver,
        Err(_) => false,
    }
}

/// Reject updates that would overwrite an active plugin/module lockfile entry.
fn ensure_not_active(entry: &LockfileEntry, pkg_id: &str) -> Result<()> {
    if entry.activation.is_some() {
        bail!(
            "Package '{}' is currently active as a plugin. \
             Deactivate it first, then run update again.",
            pkg_id
        );
    }
    if entry.module_activation.is_some() {
        bail!(
            "Package '{}' is currently active as a module. \
             Run `numan deactivate {}` first, then run update again.",
            pkg_id,
            pkg_id
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::package::ModuleImportMode;
    use crate::state::lockfile::{LockfileEntry, ModuleActivation, PluginActivation};
    use std::collections::BTreeMap;

    fn base_entry() -> LockfileEntry {
        LockfileEntry {
            version: "1.0.0".to_string(),
            package_type: "plugin".to_string(),
            source: "binary".to_string(),
            target: None,
            artifact_url: None,
            artifact_sha256: None,
            executable_path: None,
            archive_root: None,
            include: None,
            entry: None,
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
            payload_path: String::new(),
            revision_id: None,
            payload_sha256: None,
            executable_sha256: None,
            selection_reason: None,
            origin: None,
            module_activation: None,
            module_import_mode: None,
            locked_dependencies: BTreeMap::new(),
        }
    }

    #[test]
    fn is_upgrade_available_requires_strictly_newer_semver() {
        let v2 = semver::Version::new(2, 0, 0);
        assert!(is_upgrade_available("1.0.0", &v2));
        assert!(!is_upgrade_available("2.0.0", &v2));
        assert!(!is_upgrade_available("3.0.0", &v2));
        assert!(!is_upgrade_available("not-a-version", &v2));
    }

    #[test]
    fn registry_name_for_entry_prefers_origin() {
        let entry = LockfileEntry {
            origin: Some("registry:community".to_string()),
            registry_url: Some("registry:official".to_string()),
            ..base_entry()
        };
        assert_eq!(registry_name_for_entry(&entry, "official"), "community");
    }

    #[test]
    fn registry_name_for_entry_falls_back_to_default() {
        let entry = base_entry();
        assert_eq!(registry_name_for_entry(&entry, "official"), "official");
    }

    #[test]
    fn ensure_not_active_rejects_plugin_activation() {
        let entry = LockfileEntry {
            activation: Some(PluginActivation {
                plugin_registry_path: "/tmp/plugins.nu".to_string(),
                nu_executable_sha256: "abc".to_string(),
                nu_version: "0.95.0".to_string(),
                activated_at: "0".to_string(),
            }),
            ..base_entry()
        };
        let err = ensure_not_active(&entry, "owner/pkg").unwrap_err();
        assert!(err.to_string().contains("active as a plugin"));
    }

    #[test]
    fn ensure_not_active_rejects_module_activation() {
        let entry = LockfileEntry {
            module_activation: Some(ModuleActivation {
                entry_path: "/tmp/mod.nu".to_string(),
                import_mode: ModuleImportMode::Module,
                vendor_autoload_dir: "/tmp/vendor".to_string(),
                managed_file_path: "/tmp/vendor/numan.nu".to_string(),
                nu_executable_sha256: "abc".to_string(),
                nu_version: "0.95.0".to_string(),
                activated_at: "0".to_string(),
            }),
            ..base_entry()
        };
        let err = ensure_not_active(&entry, "owner/mod").unwrap_err();
        assert!(err.to_string().contains("active as a module"));
    }
}
