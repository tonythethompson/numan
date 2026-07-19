use crate::core::nu_version::NuVersion;
use crate::core::platform::Platform;
use crate::core::registry::RegistryManager;
use crate::core::resolve::Resolver;
use crate::nu::paths::NuPaths;
use anyhow::Result;
use std::path::Path;

pub fn execute(id: &str, root: &Path) -> Result<()> {
    let pkg = RegistryManager::new(root)?
        .find_package(id)?
        .ok_or_else(|| anyhow::anyhow!("Package '{id}' not found."))?;

    let platform = Platform::detect();
    let nu = current_nu(root);
    let resolver = nu.as_ref().map(|n| Resolver::new(&platform, n));

    println!("Package:    {}/{}", pkg.id.owner, pkg.id.name);
    println!("Type:       {}", pkg.package_type);
    println!("Description: {}", pkg.description);
    println!("Repository: {}", pkg.repo);
    if !pkg.tags.is_empty() {
        println!("Tags:       {}", pkg.tags.join(", "));
    }
    if let Some(ref n) = nu {
        println!("Your Nu:    {} ({})", n.version, platform.triple);
    }

    println!("\nVersions:");
    for ver in &pkg.versions {
        let status = match resolver.as_ref() {
            Some(r) => match r.classify_version(ver) {
                None => "compatible".to_string(),
                Some(issue) => issue.short_label(),
            },
            None => {
                if ver.artifact.kind == "binary"
                    && !ver.artifact.targets.contains_key(&platform.triple)
                {
                    format!("no artifact for {}", platform.triple)
                } else {
                    "nu unknown".to_string()
                }
            }
        };

        println!("  v{}  [nu {}]  ({status})", ver.version, ver.nu_version);

        if !ver.verified_with.is_empty() {
            println!("    tested with: {}", ver.verified_with.join(", "));
        }

        for target in ver.artifact.targets.keys() {
            println!("    platform: {target}");
        }
    }

    Ok(())
}

fn current_nu(root: &Path) -> Option<NuVersion> {
    if let Ok(paths) = NuPaths::load(root) {
        if let Ok(nu) = NuVersion::parse(&paths.nu_version) {
            return Some(nu);
        }
    }
    NuVersion::detect().ok()
}
