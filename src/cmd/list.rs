use crate::nu::paths::NuPaths;
use crate::nupm_compat::schema::NUPM_IMPORT_ORIGIN;
use crate::state::lockfile::Lockfile;
use anyhow::Result;
use std::path::Path;

pub fn execute(root: &Path) -> Result<()> {
    let lockfile = Lockfile::load(root)?;
    let nu_paths = NuPaths::load(root).ok();

    if lockfile.is_empty() {
        println!("No packages installed.");
        return Ok(());
    }

    println!("Installed packages ({}):\n", lockfile.packages.len());

    for (id, entry) in &lockfile.packages {
        let status = match &nu_paths {
            Some(p)
                if entry.is_active_for(
                    &p.nu_executable_hash,
                    &p.nu_version,
                    &p.plugin_registry_path,
                ) =>
            {
                "activated"
            }
            _ => "installed",
        };
        let origin_tag = if entry.origin.as_deref() == Some(NUPM_IMPORT_ORIGIN) {
            " (nupm import)"
        } else {
            ""
        };
        println!(
            "  {}  v{}  [{}]  {}{}",
            id, entry.version, entry.package_type, status, origin_tag
        );
    }

    Ok(())
}
