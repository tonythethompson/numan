use anyhow::{bail, Result};
use clap::Subcommand;
use crate::core::registry::RegistryManager;
use crate::core::trust::TrustStore;
use std::path::PathBuf;

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

pub fn execute(cmd: RegistryCommands, root: &PathBuf) -> Result<()> {
    match cmd {
        RegistryCommands::List => list_registries(root),
        RegistryCommands::Sync => sync_registries(root),
        RegistryCommands::Add { name, url, key } => add_registry(root, &name, &url, &key),
        RegistryCommands::Remove { name } => remove_registry(root, &name),
        RegistryCommands::Packages => list_packages(root),
    }
}

fn list_registries(root: &PathBuf) -> Result<()> {
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

fn sync_registries(root: &PathBuf) -> Result<()> {
    let config = crate::config::Config::load(root)?;
    let _mgr = RegistryManager::new(root)?;

    for (name, reg) in &config.registries {
        if !reg.enabled {
            continue;
        }

        println!("Syncing '{name}' from {}...", reg.url);

        // For now, just download the index.json from the URL
        let response = reqwest::blocking::get(&reg.url)
            .map_err(|e| anyhow::anyhow!("Failed to fetch registry '{name}': {e}"))?;

        if !response.status().is_success() {
            bail!("Failed to fetch registry '{name}': HTTP {}", response.status());
        }

        let index_content = response.text()?;

        // Save the index
        let index_path = root.join(format!("registry/{name}/index.json"));
        if let Some(parent) = index_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&index_path, &index_content)?;

        // Try to fetch signature
        let sig_url = format!("{}.sig", reg.url);
        if let Ok(sig_response) = reqwest::blocking::get(&sig_url) {
            if sig_response.status().is_success() {
                let sig_path = root.join(format!("registry/{name}/index.json.sig"));
                std::fs::write(sig_path, sig_response.text()?)?;
            }
        }

        println!("  Synced '{name}' successfully.");
    }

    Ok(())
}

fn add_registry(root: &PathBuf, name: &str, url: &str, key_b64: &str) -> Result<()> {
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

fn remove_registry(root: &PathBuf, name: &str) -> Result<()> {
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

fn list_packages(root: &PathBuf) -> Result<()> {
    let config = crate::config::Config::load(root)?;
    let mgr = RegistryManager::new(root)?;

    let default_reg = &config.general.default_registry;
    let index = mgr.load_index(default_reg)?;

    println!("Packages in '{default_reg}' ({}):\n", index.packages.len());

    for pkg in &index.packages {
        let latest = pkg.versions.last().map(|v| v.version.to_string()).unwrap_or_else(|| "n/a".to_string());
        println!(
            "  {}/{}  v{}  [{}]
    {}",
            pkg.id.owner,
            pkg.id.name,
            latest,
            pkg.package_type,
            pkg.description
        );
    }

    Ok(())
}
