//! Shared single-plugin activate/deactivate helpers (Issue #22 PR3).
//!
//! Used by `update` orchestration while the caller already holds the mutation
//! lock. These helpers intentionally do **not** acquire `mutation.lock`, print
//! consent tables, or create snapshots (callers own those boundaries).
//!
//! Full CLI flows remain in [`super::activate`] and [`super::deactivate`].

use anyhow::{bail, Context, Result};
use std::path::Path;

use crate::cmd::activate::validate_payload_path;
use crate::cmd::deactivate::plugin_name_from_executable_path;
use crate::nu::paths::NuPaths;
use crate::state::journal::{PendingActivation, PendingActivationEntry, PendingStatus};
use crate::state::lockfile::{Lockfile, PluginActivation};
use crate::state::plugin_deactivate_journal::{
    PendingPluginDeactivate, PendingPluginDeactivateEntry, PluginDeactivateStatus,
};
use crate::util::format_timestamp;

/// Unregister one active plugin and clear its lockfile `activation` record.
///
/// Caller must hold the root mutation lock. Idempotent when already inactive.
pub fn deactivate_one_plugin(
    root: &Path,
    pkg_id: &str,
    unregistrar: &dyn Fn(&str, &str, &str) -> Result<()>,
) -> Result<()> {
    let nu_paths = NuPaths::load(root)?;
    nu_paths.validate_drift()?;
    let mut lockfile = Lockfile::load(root)?;

    let entry = lockfile
        .packages
        .get(pkg_id)
        .with_context(|| format!("Package '{pkg_id}' is not installed"))?;

    if entry.package_type != "plugin" {
        bail!("Package '{pkg_id}' is not a plugin (type: {})", entry.package_type);
    }

    if entry.activation.is_none() {
        return Ok(());
    }

    let executable_path = entry
        .executable_path
        .as_deref()
        .with_context(|| format!("{pkg_id}: missing executable_path in lockfile"))?
        .to_string();
    let plugin_name = plugin_name_from_executable_path(&executable_path);
    let absolute_binary_path = root.join(&entry.payload_path).join(&executable_path);

    let mut journal = PendingPluginDeactivate {
        nu_executable_sha256: nu_paths.nu_executable_hash.clone(),
        nu_version: nu_paths.nu_version.clone(),
        plugin_registry_path: nu_paths.plugin_registry_path.clone(),
        created_at: format_timestamp(),
        entries: vec![PendingPluginDeactivateEntry {
            package_id: pkg_id.to_string(),
            plugin_name: plugin_name.clone(),
            absolute_binary_path: absolute_binary_path.to_string_lossy().into_owned(),
            status: PluginDeactivateStatus::Prepared,
            error: None,
        }],
    };
    journal.save(root)?;

    match unregistrar(
        &nu_paths.nu_executable,
        &plugin_name,
        &nu_paths.plugin_registry_path,
    ) {
        Ok(()) => {
            journal.entries[0].status = PluginDeactivateStatus::Unregistered;
            journal.save(root)?;

            if let Some(pkg) = lockfile.packages.get_mut(pkg_id) {
                pkg.activation = None;
            }
            lockfile.save(root)?;
            PendingPluginDeactivate::delete(root)?;
            Ok(())
        }
        Err(e) => {
            journal.entries[0].status = PluginDeactivateStatus::Failed;
            journal.entries[0].error = Some(e.to_string());
            journal.save(root)?;
            Err(e).with_context(|| format!("Failed to unregister active plugin '{pkg_id}'"))
        }
    }
}

/// Register one inactive plugin and write its lockfile `activation` record.
///
/// Caller must hold the root mutation lock. Fails if the package is already
/// active for the current Nu identity.
pub fn activate_one_plugin(
    root: &Path,
    pkg_id: &str,
    registrar: &dyn Fn(&str, &str, &str) -> Result<()>,
) -> Result<()> {
    let nu_paths = NuPaths::load(root)?;
    nu_paths.validate_drift()?;
    let mut lockfile = Lockfile::load(root)?;

    let entry = lockfile
        .packages
        .get(pkg_id)
        .with_context(|| format!("Package '{pkg_id}' is not installed"))?;

    if entry.package_type != "plugin" {
        bail!("Package '{pkg_id}' is not a plugin (type: {})", entry.package_type);
    }

    if entry.is_active_for(
        &nu_paths.nu_executable_hash,
        &nu_paths.nu_version,
        &nu_paths.plugin_registry_path,
    ) {
        return Ok(());
    }

    let executable_path = entry
        .executable_path
        .as_deref()
        .with_context(|| format!("{pkg_id}: missing executable_path in lockfile"))?
        .to_string();
    let payload_path = entry.payload_path.clone();
    let absolute_binary_path = validate_payload_path(root, &payload_path, &executable_path)?;
    let binary_str = absolute_binary_path.to_string_lossy().into_owned();

    let mut journal = PendingActivation {
        nu_executable_sha256: nu_paths.nu_executable_hash.clone(),
        nu_version: nu_paths.nu_version.clone(),
        plugin_registry_path: nu_paths.plugin_registry_path.clone(),
        created_at: format_timestamp(),
        entries: vec![PendingActivationEntry {
            package_id: pkg_id.to_string(),
            payload_path: payload_path.clone(),
            executable_path: executable_path.clone(),
            absolute_binary_path: binary_str.clone(),
            status: PendingStatus::Prepared,
            error: None,
        }],
    };
    journal.save(root)?;

    match registrar(
        &nu_paths.nu_executable,
        &binary_str,
        &nu_paths.plugin_registry_path,
    ) {
        Ok(()) => {
            journal.entries[0].status = PendingStatus::Registered;
            journal.save(root)?;

            if let Some(pkg) = lockfile.packages.get_mut(pkg_id) {
                pkg.activation = Some(PluginActivation {
                    plugin_registry_path: nu_paths.plugin_registry_path.clone(),
                    nu_executable_sha256: nu_paths.nu_executable_hash.clone(),
                    nu_version: nu_paths.nu_version.clone(),
                    activated_at: format_timestamp(),
                });
            }
            lockfile.save(root)?;
            PendingActivation::delete(root)?;
            Ok(())
        }
        Err(e) => {
            journal.entries[0].status = PendingStatus::Failed;
            journal.entries[0].error = Some(e.to_string());
            journal.save(root)?;
            Err(e).with_context(|| format!("Failed to register plugin '{pkg_id}' after update"))
        }
    }
}

/// Production Nu `plugin rm` seam (name/config via env only).
pub fn run_plugin_rm(nu_executable: &str, plugin_name: &str, plugin_config: &str) -> Result<()> {
    crate::cmd::deactivate::run_plugin_rm(nu_executable, plugin_name, plugin_config)
}

/// Production Nu `plugin add` seam (binary/config via env only).
pub fn run_plugin_add(nu_executable: &str, plugin_binary: &str, plugin_config: &str) -> Result<()> {
    crate::cmd::activate::run_plugin_add(nu_executable, plugin_binary, plugin_config)
}
