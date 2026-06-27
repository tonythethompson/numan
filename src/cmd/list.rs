use anyhow::Result;
use crate::state::lockfile::Lockfile;
use std::path::Path;

pub fn execute(root: &Path) -> Result<()> {
    let lockfile = Lockfile::load(root)?;

    if lockfile.is_empty() {
        println!("No packages installed.");
        return Ok(());
    }

    println!("Installed packages ({}):\n", lockfile.packages.len());

    for (id, entry) in &lockfile.packages {
        let status = if entry.activation.is_some() {
            "activated"
        } else {
            "installed"
        };
        println!(
            "  {}  v{}  [{}]  {}",
            id, entry.version, entry.package_type, status
        );
    }

    Ok(())
}
