use anyhow::Result;
use crate::core::registry::RegistryManager;
use crate::core::platform::Platform;
use crate::core::nu_version::NuVersion;
use std::path::Path;

pub fn execute(id: &str, root: &Path) -> Result<()> {
    let pkg = RegistryManager::new(root)?
        .find_package(id)?
        .ok_or_else(|| anyhow::anyhow!("Package '{id}' not found."))?;

    let platform = Platform::detect();
    let nu_version = NuVersion::detect().ok();

    println!("Package:    {}/{}", pkg.id.owner, pkg.id.name);
    println!("Type:       {}", pkg.package_type);
    println!("Description: {}", pkg.description);
    println!("Repository: {}", pkg.repo);
    if !pkg.tags.is_empty() {
        println!("Tags:       {}", pkg.tags.join(", "));
    }

    println!("\nVersions:");
    for ver in &pkg.versions {
        let mut flags = vec![];
        if let Some(ref nu) = nu_version {
            if nu.matches_constraint(&ver.nu_version) {
                flags.push("compatible".to_string());
            }
        }
        if ver.artifact.targets.contains_key(&platform.triple) {
            flags.push(format!("platform:{}", platform.triple));
        }

        let flag_str = if flags.is_empty() {
            String::new()
        } else {
            format!(" ({})", flags.join(", "))
        };

        println!(
            "  v{}  [nu {}]{}",
            ver.version, ver.nu_version, flag_str
        );

        if !ver.verified_with.is_empty() {
            println!("    tested with: {}", ver.verified_with.join(", "));
        }

        for target in ver.artifact.targets.keys() {
            println!("    platform: {target}");
        }
    }

    Ok(())
}
