use anyhow::Result;
use crate::state::lockfile::Lockfile;
use std::path::PathBuf;

pub fn execute(root: &PathBuf) -> Result<()> {
    let lockfile = Lockfile::load(root)?;

    if lockfile.is_empty() {
        println!("No packages installed.");
        return Ok(());
    }

    println!("Installed packages ({}):\n", lockfile.packages.len());

    for (id, entry) in &lockfile.packages {
        let status = if entry.activated { "activated" } else { "installed" };
        println!(
            "  {}  v{}  [{}]  {}",
            id, entry.version, entry.package_type, status
        );
    }

    Ok(())
}
