use crate::core::registry::RegistryManager;
use anyhow::Result;
use std::path::Path;

pub fn execute(query: &str, root: &Path) -> Result<()> {
    let mgr = RegistryManager::new(root)?;
    let results = mgr.search(query)?;

    if results.is_empty() {
        println!("No packages found matching '{query}'.");
        return Ok(());
    }

    println!("Found {} package(s) matching '{}':\n", results.len(), query);

    for pkg in &results {
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
