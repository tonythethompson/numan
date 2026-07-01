use crate::core::official_registry::RegistrySignature;
use crate::core::registry::{RegistryManager, VerifiedRegistry};
use crate::core::trust::TrustStore;
use anyhow::{bail, Context, Result};
use clap::Subcommand;
use std::path::Path;

#[derive(Subcommand)]
pub enum RegistryCommands {
    /// List configured registries
    List,
    /// Fetch latest index from all registries
    Sync,
    /// Add a custom registry
    Add {
        /// Registry name
        name: String,
        /// Registry index URL
        url: String,
        /// Ed25519 public key (base64)
        #[arg(long)]
        key: String,
    },
    /// Remove a registry
    Remove {
        /// Registry name
        name: String,
    },
    /// List all packages in registry
    Packages,
}

pub fn execute(cmd: RegistryCommands, root: &Path) -> Result<()> {
    match cmd {
        RegistryCommands::List => list_registries(root),
        RegistryCommands::Sync => sync_registries(root),
        RegistryCommands::Add { name, url, key } => add_registry(root, &name, &url, &key),
        RegistryCommands::Remove { name } => remove_registry(root, &name),
        RegistryCommands::Packages => list_packages(root),
    }
}

fn list_registries(root: &Path) -> Result<()> {
    let config = crate::config::Config::load(root)?;
    if config.registries.is_empty() {
        println!("No registries configured.");
        return Ok(());
    }

    println!("Configured registries:\n");
    for (name, reg) in &config.registries {
        let status = if reg.enabled { "enabled" } else { "disabled" };
        println!("  {name}  [{status}]");
        println!("    url: {}", reg.url);
    }

    Ok(())
}

fn sync_registries(root: &Path) -> Result<()> {
    let config = crate::config::Config::load(root)?;
    let mgr = RegistryManager::new(root)?;

    for (name, reg) in &config.registries {
        if !reg.enabled {
            continue;
        }

        println!("Syncing '{name}' from {}...", reg.url);

        let fetch_result: Result<VerifiedRegistry> = (|| {
            let index_response = reqwest::blocking::get(&reg.url)
                .and_then(|r| r.error_for_status())
                .map_err(|e| anyhow::anyhow!("Failed to fetch registry '{name}': {e}"))?;
            let sig_response = reqwest::blocking::get(format!("{}.sig", reg.url))
                .and_then(|r| r.error_for_status())
                .map_err(|e| anyhow::anyhow!("Failed to fetch signature for '{name}': {e}"))?;

            let index_content = index_response.text()?;
            let sig_content = sig_response.text()?;
            let signature: RegistrySignature = serde_json::from_str(&sig_content)
                .with_context(|| format!("Registry '{name}' signature file is malformed"))?;
            mgr.replace_index(name, &index_content, &signature)
        })();

        let verified = match fetch_result {
            Ok(v) => v,
            Err(e) => {
                // Network fetch or signature validation failed. If a cached
                // verified index exists, use it and warn; otherwise error.
                let cached = mgr.load_verified(name);
                if let Ok(cached) = cached {
                    eprintln!(
                        "Warning: Could not refresh registry '{name}' ({e}); using cached index from {}.",
                        cached.index.updated_at
                    );
                    cached
                } else if let Ok(lkg) = mgr.load_last_known_good(name) {
                    eprintln!(
                        "Warning: Could not refresh registry '{name}' ({e}); using last-known-good index from {}.",
                        lkg.index.updated_at
                    );
                    lkg
                } else {
                    bail!("Failed to sync registry '{name}' and no cached or last-known-good index is available: {e}");
                }
            }
        };

        println!(
            "  Synced '{name}' successfully (key_id: {}, index_sha256: {}).",
            verified.key_id,
            &verified.index_sha256[..8.min(verified.index_sha256.len())]
        );
    }

    Ok(())
}

fn add_registry(root: &Path, name: &str, url: &str, key_b64: &str) -> Result<()> {
    let mut config = crate::config::Config::load(root)?;

    if config.registries.contains_key(name) {
        bail!("Registry '{name}' already exists. Remove it first.");
    }

    // Add key to trust store
    let mut trust = TrustStore::load(root)?;
    let fingerprint = trust.add_key(name, key_b64)?;
    trust.save(root)?;

    // Add registry to config
    config.registries.insert(
        name.to_string(),
        crate::config::RegistryConfig {
            url: url.to_string(),
            sync_interval: "24h".to_string(),
            enabled: true,
            trust_key: Some(key_b64.to_string()),
        },
    );
    config.save(root)?;

    println!("Added registry '{name}'.");
    println!("  URL: {url}");
    println!("  Fingerprint: {fingerprint}");
    println!("\nRun 'numan registry sync' to fetch the index.");

    Ok(())
}

fn remove_registry(root: &Path, name: &str) -> Result<()> {
    let mut config = crate::config::Config::load(root)?;

    if !config.registries.contains_key(name) {
        bail!("Registry '{name}' not found.");
    }

    config.registries.remove(name);
    config.save(root)?;

    // Remove cached index
    let index_dir = root.join(format!("registry/{name}"));
    if index_dir.exists() {
        std::fs::remove_dir_all(index_dir)?;
    }

    println!("Removed registry '{name}'.");
    Ok(())
}

fn list_packages(root: &Path) -> Result<()> {
    let config = crate::config::Config::load(root)?;
    let mgr = RegistryManager::new(root)?;

    let default_reg = &config.general.default_registry;
    let index = mgr.load_index(default_reg)?;

    println!("Packages in '{default_reg}' ({}):\n", index.packages.len());

    for pkg in &index.packages {
        let latest = pkg
            .versions
            .last()
            .map(|v| v.version.to_string())
            .unwrap_or_else(|| "n/a".to_string());
        println!(
            "  {}/{}  v{}  [{}]
    {}",
            pkg.id.owner, pkg.id.name, latest, pkg.package_type, pkg.description
        );
    }

    Ok(())
}
