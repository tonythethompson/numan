use anyhow::{bail, Result};
use clap::Parser;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::cmd::plugin_lifecycle::{
    activate_one_plugin, deactivate_one_plugin, run_plugin_add, run_plugin_rm,
};
use crate::core::nu_version::NuVersion;
use crate::core::package::RegistryIndex;
use crate::core::platform::Platform;
use crate::core::registry::RegistryManager;
use crate::core::resolve::Resolver;
use crate::install::transaction::{self, InstallOptions, InstallResult};
use crate::state::active_plugin_mutation;
use crate::state::lifecycle_journal::{
    check_stale_journal, LifecycleOp, LifecycleStage, PendingLifecycle,
};
use crate::state::lockfile::{Lockfile, LockfileEntry};
use crate::util::fs_safety::acquire_mutation_lock;
use crate::util::hints;

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

/// How an active lockfile entry should be handled during update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveUpdatePlan {
    /// No plugin/module activation; plain upgrade.
    Plain,
    /// Plugin activation present and mutation flag enabled: deactivate → install → activate.
    OrchestrateActivePlugin,
}

/// Injectable install seam for update tests (production uses `transaction::install_package`).
pub type UpdateInstallFn =
    dyn Fn(&str, Option<&str>, &InstallOptions<'_>) -> Result<InstallResult>;

/// Injectable seams for active-plugin update orchestration (tests inject fakes).
pub struct UpdateHooks<'a> {
    pub unregistrar: &'a dyn Fn(&str, &str, &str) -> Result<()>,
    pub registrar: &'a dyn Fn(&str, &str, &str) -> Result<()>,
    /// When `None`, uses [`transaction::install_package`].
    pub install: Option<&'a UpdateInstallFn>,
}

impl Default for UpdateHooks<'static> {
    fn default() -> Self {
        Self {
            unregistrar: &run_plugin_rm,
            registrar: &run_plugin_add,
            install: None,
        }
    }
}

pub fn execute(args: &UpdateArgs, root: &PathBuf) -> Result<()> {
    execute_with_hooks(args, root, &UpdateHooks::default())
}

/// Testability entry: inject deactivate/activate/install seams.
pub fn execute_with_hooks(args: &UpdateArgs, root: &PathBuf, hooks: &UpdateHooks<'_>) -> Result<()> {
    if let Some(journal) = check_stale_journal(root)? {
        let op = match journal.op {
            LifecycleOp::Update => "update",
            LifecycleOp::Remove => "remove",
            LifecycleOp::NupmImport => "nupm import",
            LifecycleOp::NupmImportManifest => "nupm manifest import",
            LifecycleOp::Rollback => "rollback",
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

        let plan = match plan_active_update(current, &update.package_id) {
            Ok(plan) => plan,
            Err(e) => {
                if args.package.is_some() {
                    return Err(e);
                }
                eprintln!("  {e}");
                failures.push(update.package_id.clone());
                continue;
            }
        };

        let journal = PendingLifecycle {
            op: LifecycleOp::Update,
            package_id: update.package_id.clone(),
            stage: LifecycleStage::Prepared,
            orphan_payload_path: Some(update.orphan_path.clone()),
            from_version: Some(update.from_version.clone()),
            to_version: Some(update.to_version.clone()),
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

        if plan == ActiveUpdatePlan::OrchestrateActivePlugin {
            if args.verbose {
                println!(
                    "  Deactivating {} before upgrade (active-plugin update)",
                    update.package_id
                );
            }
            if let Err(e) =
                deactivate_one_plugin(root, &update.package_id, hooks.unregistrar)
            {
                eprintln!("Failed to deactivate {}: {}", update.package_id, e);
                // Leave PendingLifecycle at Prepared + deactivate journal for recovery.
                failures.push(update.package_id.clone());
                continue;
            }
        }

        let options = InstallOptions {
            root,
            platform: &platform,
            nu_version: &nu_version,
            force: false,
            verbose: args.verbose,
            registry_name: Some(&update.registry_name),
            snapshot_trigger: crate::state::snapshot::SnapshotTrigger::Update,
        };

        let install_result = match hooks.install {
            Some(install) => install(&update.package_id, Some(&update.to_version), &options),
            None => transaction::install_package(
                &update.package_id,
                Some(&update.to_version),
                &options,
            ),
        };

        match install_result {
            Ok(_) => {
                PendingLifecycle {
                    stage: LifecycleStage::LockfileUpdated,
                    ..journal.clone()
                }
                .save(root)?;

                if plan == ActiveUpdatePlan::OrchestrateActivePlugin {
                    if args.verbose {
                        println!(
                            "  Reactivating {} after upgrade (active-plugin update)",
                            update.package_id
                        );
                    }
                    if let Err(e) =
                        activate_one_plugin(root, &update.package_id, hooks.registrar)
                    {
                        eprintln!(
                            "Failed to reactivate {} after upgrade: {}",
                            update.package_id, e
                        );
                        eprintln!(
                            "Package was upgraded but is inactive. Run `numan activate {} --yes`.",
                            update.package_id
                        );
                        // Leave LockfileUpdated journal + activate journal for recovery.
                        failures.push(update.package_id.clone());
                        continue;
                    }
                }

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
                // If we already deactivated, package is inactive; lifecycle journal remains.
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

/// Decide whether an active plugin/module may be updated.
///
/// - Active **module**: always refuse (deactivate first).
/// - Active **plugin** with mutation disabled: refuse (env kill switch / manual deactivate).
/// - Active **plugin** with mutation enabled: orchestrate deactivate → install → activate.
fn plan_active_update(entry: &LockfileEntry, pkg_id: &str) -> Result<ActiveUpdatePlan> {
    if entry.module_activation.is_some() {
        bail!(
            "Package '{}' is currently active as a module. \
             Run `numan deactivate {}` first, then run update again.",
            pkg_id,
            pkg_id
        );
    }
    if entry.activation.is_some() {
        if !active_plugin_mutation::is_enabled() {
            bail!("{}", hints::active_plugin_update_disabled(pkg_id));
        }
        return Ok(ActiveUpdatePlan::OrchestrateActivePlugin);
    }
    Ok(ActiveUpdatePlan::Plain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::integrity;
    use crate::core::package::ModuleImportMode;
    use crate::nu::paths::NuPaths;
    use crate::state::lifecycle_journal::PendingLifecycle;
    use crate::state::lockfile::{Lockfile, LockfileEntry, ModuleActivation, PluginActivation};
    use crate::state::plugin_deactivate_journal::PendingPluginDeactivate;
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use tempfile::TempDir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

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

    fn plugin_activation() -> PluginActivation {
        PluginActivation {
            plugin_registry_path: "/tmp/plugins.nu".to_string(),
            nu_executable_sha256: "abc".to_string(),
            nu_version: "0.95.0".to_string(),
            activated_at: "0".to_string(),
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
    fn plan_active_update_orchestrates_when_enabled() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION");
        let entry = LockfileEntry {
            activation: Some(plugin_activation()),
            ..base_entry()
        };
        assert_eq!(
            plan_active_update(&entry, "owner/pkg").unwrap(),
            ActiveUpdatePlan::OrchestrateActivePlugin
        );
    }

    #[test]
    fn plan_active_update_refuses_when_mutation_disabled() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION", "0");
        let entry = LockfileEntry {
            activation: Some(plugin_activation()),
            ..base_entry()
        };
        let err = plan_active_update(&entry, "owner/pkg").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("owner/pkg"));
        assert!(msg.contains("NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION"));
        std::env::remove_var("NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION");
    }

    #[test]
    fn plan_active_update_rejects_module_activation() {
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
        let err = plan_active_update(&entry, "owner/mod").unwrap_err();
        assert!(err.to_string().contains("active as a module"));
    }

    struct ActiveUpdateEnv {
        _dir: TempDir,
        root: PathBuf,
        pkg_id: String,
    }

    impl ActiveUpdateEnv {
        fn new() -> Self {
            let dir = TempDir::new().unwrap();
            let root = dir.path().to_path_buf();
            std::fs::create_dir_all(root.join("state")).unwrap();
            std::fs::create_dir_all(root.join("packages")).unwrap();

            let nu_exe = root.join("fake_nu");
            std::fs::write(&nu_exe, b"fake nu binary").unwrap();
            let nu_hash = integrity::compute_sha256(b"fake nu binary");
            let registry = root.join("plugin-registry.msgpack.z");
            std::fs::write(&registry, b"reg").unwrap();

            let paths = NuPaths {
                nu_executable: nu_exe.to_string_lossy().into_owned(),
                nu_version: "0.95.0".to_string(),
                plugin_registry_path: registry.to_string_lossy().into_owned(),
                nu_executable_hash: nu_hash.clone(),
                platform: "x86_64-pc-windows-msvc".to_string(),
                data_dir: None,
                vendor_autoload_dirs: vec![],
                vendor_autoload_dir: None,
            };
            paths.save(&root).unwrap();

            let pkg_id = "owner/highlight".to_string();
            let payload = "packages/plugins/owner/highlight/1.0.0-abc";
            let payload_dir = root.join(payload);
            std::fs::create_dir_all(&payload_dir).unwrap();
            std::fs::write(payload_dir.join("nu_plugin_highlight"), b"fake").unwrap();

            let mut lockfile = Lockfile::empty();
            lockfile.packages.insert(
                pkg_id.clone(),
                LockfileEntry {
                    version: "1.0.0".to_string(),
                    package_type: "plugin".to_string(),
                    source: "binary".to_string(),
                    target: None,
                    artifact_url: None,
                    artifact_sha256: None,
                    executable_path: Some("nu_plugin_highlight".to_string()),
                    archive_root: None,
                    include: None,
                    entry: None,
                    installed_at: "0".to_string(),
                    nu_version_at_install: None,
                    activation: Some(PluginActivation {
                        plugin_registry_path: paths.plugin_registry_path.clone(),
                        nu_executable_sha256: nu_hash,
                        nu_version: paths.nu_version.clone(),
                        activated_at: "0".to_string(),
                    }),
                    registry_url: None,
                    registry_revision: None,
                    index_sha256: None,
                    signing_key_fingerprint: None,
                    git_url: None,
                    git_rev: None,
                    cargo_name: None,
                    cargo_lock_sha256: None,
                    built_sha256: None,
                    payload_path: payload.to_string(),
                    revision_id: None,
                    payload_sha256: None,
                    executable_sha256: None,
                    selection_reason: None,
                    origin: Some("registry:official".to_string()),
                    module_activation: None,
                    module_import_mode: None,
                    locked_dependencies: BTreeMap::new(),
                },
            );
            lockfile.save(&root).unwrap();

            Self {
                _dir: dir,
                root,
                pkg_id,
            }
        }

        fn root(&self) -> &PathBuf {
            &self.root
        }
    }

    /// Bypass registry discovery: one fabricated pending update, then hooks.
    fn run_orchestrated_update_with_hooks(
        env: &ActiveUpdateEnv,
        unregistrar: &dyn Fn(&str, &str, &str) -> Result<()>,
        registrar: &dyn Fn(&str, &str, &str) -> Result<()>,
        install: &dyn Fn(&str, Option<&str>, &InstallOptions<'_>) -> Result<InstallResult>,
    ) -> Result<()> {
        let _lock = acquire_mutation_lock(env.root())?;
        let lockfile = Lockfile::load(env.root())?;
        let entry = lockfile.packages.get(&env.pkg_id).unwrap();
        let plan = plan_active_update(entry, &env.pkg_id)?;
        assert_eq!(plan, ActiveUpdatePlan::OrchestrateActivePlugin);

        let journal = PendingLifecycle {
            op: LifecycleOp::Update,
            package_id: env.pkg_id.clone(),
            stage: LifecycleStage::Prepared,
            orphan_payload_path: Some(entry.payload_path.clone()),
            from_version: Some("1.0.0".to_string()),
            to_version: Some("2.0.0".to_string()),
            nupm_source_path: None,
            nupm_metadata_sha256: None,
            staging_dir: None,
            promoted_payload_path: None,
            batch_package_ids: Vec::new(),
            batch_staging_dirs: Vec::new(),
            target_snapshot_id: None,
            pre_rollback_snapshot_id: None,
        };
        journal.save(env.root())?;

        deactivate_one_plugin(env.root(), &env.pkg_id, unregistrar)?;

        let platform = Platform::detect();
        let nu_version = NuVersion {
            version: "0.95.0".to_string(),
            major: 0,
            minor: 95,
            patch: 0,
        };
        let options = InstallOptions {
            root: env.root(),
            platform: &platform,
            nu_version: &nu_version,
            force: false,
            verbose: false,
            registry_name: Some("official"),
            snapshot_trigger: crate::state::snapshot::SnapshotTrigger::Update,
        };
        install(&env.pkg_id, Some("2.0.0"), &options)?;

        PendingLifecycle {
            stage: LifecycleStage::LockfileUpdated,
            ..journal.clone()
        }
        .save(env.root())?;

        activate_one_plugin(env.root(), &env.pkg_id, registrar)?;
        PendingLifecycle::clear(env.root())?;
        Ok(())
    }

    #[test]
    fn active_plugin_update_succeeds_with_fake_hooks() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION");

        let env = ActiveUpdateEnv::new();
        let pkg_id = env.pkg_id.clone();
        let root = env.root().clone();

        let install = move |id: &str, ver: Option<&str>, opts: &InstallOptions<'_>| {
            assert_eq!(id, pkg_id);
            assert_eq!(ver, Some("2.0.0"));
            let mut lockfile = Lockfile::load(opts.root)?;
            let entry = lockfile.packages.get_mut(id).unwrap();
            assert!(entry.activation.is_none(), "deactivate must clear activation");
            let new_payload = "packages/plugins/owner/highlight/2.0.0-def";
            let new_dir = opts.root.join(new_payload);
            std::fs::create_dir_all(&new_dir)?;
            std::fs::write(new_dir.join("nu_plugin_highlight"), b"fake v2")?;
            entry.version = "2.0.0".to_string();
            entry.payload_path = new_payload.to_string();
            lockfile.save(opts.root)?;
            Ok(InstallResult {
                installed: true,
                package: id.to_string(),
                version: "2.0.0".to_string(),
                path: new_dir,
                already_existed: false,
            })
        };

        run_orchestrated_update_with_hooks(
            &env,
            &|_nu, name, _cfg| {
                assert_eq!(name, "highlight");
                Ok(())
            },
            &|_nu, bin, _cfg| {
                assert!(bin.contains("nu_plugin_highlight"));
                Ok(())
            },
            &install,
        )
        .unwrap();

        let lockfile = Lockfile::load(&root).unwrap();
        let entry = lockfile.packages.get(&env.pkg_id).unwrap();
        assert_eq!(entry.version, "2.0.0");
        assert!(entry.activation.is_some());
        assert!(PendingLifecycle::load(&root).unwrap().is_none());
    }

    #[test]
    fn active_plugin_update_refuses_when_env_disabled() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION", "0");

        let env = ActiveUpdateEnv::new();
        let lockfile = Lockfile::load(env.root()).unwrap();
        let entry = lockfile.packages.get(&env.pkg_id).unwrap();
        let err = plan_active_update(entry, &env.pkg_id).unwrap_err();
        assert!(err.to_string().contains("NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION"));

        std::env::remove_var("NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION");
    }

    #[test]
    fn unregistrar_failure_leaves_activation_and_journals() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION");

        let env = ActiveUpdateEnv::new();
        let _lock = acquire_mutation_lock(env.root()).unwrap();
        let lockfile = Lockfile::load(env.root()).unwrap();
        let entry = lockfile.packages.get(&env.pkg_id).unwrap();

        let journal = PendingLifecycle {
            op: LifecycleOp::Update,
            package_id: env.pkg_id.clone(),
            stage: LifecycleStage::Prepared,
            orphan_payload_path: Some(entry.payload_path.clone()),
            from_version: Some("1.0.0".to_string()),
            to_version: Some("2.0.0".to_string()),
            nupm_source_path: None,
            nupm_metadata_sha256: None,
            staging_dir: None,
            promoted_payload_path: None,
            batch_package_ids: Vec::new(),
            batch_staging_dirs: Vec::new(),
            target_snapshot_id: None,
            pre_rollback_snapshot_id: None,
        };
        journal.save(env.root()).unwrap();

        let err = deactivate_one_plugin(env.root(), &env.pkg_id, &|_nu, _name, _cfg| {
            bail!("injected unregister failure")
        })
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("injected unregister failure"),
            "unexpected error: {msg}"
        );

        let lockfile = Lockfile::load(env.root()).unwrap();
        assert!(
            lockfile.packages.get(&env.pkg_id).unwrap().activation.is_some(),
            "activation must remain when unregister fails"
        );
        assert!(PendingLifecycle::load(env.root()).unwrap().is_some());
        assert!(PendingPluginDeactivate::load(env.root()).unwrap().is_some());
    }
}
