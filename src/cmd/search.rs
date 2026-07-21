use crate::core::nu_version::NuVersion;
use crate::core::platform::Platform;
use crate::core::registry::RegistryManager;
use crate::core::resolve::Resolver;
use anyhow::Result;
use clap::Parser;
use std::path::Path;

/// Search registry by name/description/tags
#[derive(Parser, Debug)]
pub struct SearchArgs {
    /// Search query
    pub query: String,

    /// Show incompatible packages too (marked in the listing)
    #[arg(long)]
    pub all: bool,
}

pub fn execute(args: &SearchArgs, root: &Path) -> Result<()> {
    let mgr = RegistryManager::new(root)?;
    let results = mgr.search(&args.query)?;

    if results.is_empty() {
        println!("No packages found matching '{}'.", args.query);
        return Ok(());
    }

    let platform = Platform::detect();
    let nu = current_nu(root);
    let resolver = nu.as_ref().map(|n| Resolver::new(&platform, n));

    let mut shown = 0usize;
    let mut hidden = 0usize;

    println!(
        "Found {} package(s) matching '{}':\n",
        results.len(),
        args.query
    );

    for pkg in &results {
        let compatible = resolver
            .as_ref()
            .map(|r| r.has_compatible_version(pkg))
            .unwrap_or(true);

        if !compatible && !args.all {
            hidden += 1;
            continue;
        }

        shown += 1;

        // Find newest version by semver (for display and classification)
        let newest_version = pkg
            .versions
            .iter()
            .max_by(|a, b| a.version.cmp(&b.version));

        let version_label = match resolver.as_ref() {
            Some(r) if compatible => r
                .latest_compatible(pkg)
                .map(|v| v.version.to_string())
                .unwrap_or_else(|| "n/a".to_string()),
            _ => newest_version
                .map(|v| v.version.to_string())
                .unwrap_or_else(|| "n/a".to_string()),
        };

        let status = match resolver.as_ref() {
            Some(r) if args.all || !compatible => {
                if compatible {
                    " [compatible]".to_string()
                } else if let Some(entry) = newest_version {
                    match r.classify_version(entry) {
                        Some(issue) => format!(" [{}]", issue.short_label()),
                        None => " [incompatible]".to_string(),
                    }
                } else {
                    " [incompatible]".to_string()
                }
            }
            _ => String::new(),
        };

        println!(
            "  {}/{}  v{}  [{}]{}
    {}",
            pkg.id.owner, pkg.id.name, version_label, pkg.package_type, status, pkg.description
        );
    }

    if shown == 0 && hidden > 0 {
        println!("(no compatible packages for your Nu/platform; {hidden} match(es) hidden)");
    }

    if hidden > 0 {
        let nu_label = nu.as_ref().map(|n| n.version.as_str()).unwrap_or("unknown");
        println!(
            "\n{hidden} package(s) hidden (incompatible with Nu {nu_label} / {}). Use --all to show them.",
            platform.triple
        );
    }

    Ok(())
}

fn current_nu(root: &Path) -> Option<NuVersion> {
    NuVersion::from_paths_or_detect(root).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::package::*;
    use crate::core::resolve::Resolver;
    use std::collections::{BTreeMap, HashMap};

    fn sample_pkg(nu_constraint: &str) -> Package {
        let mut targets = HashMap::new();
        targets.insert(
            "x86_64-unknown-linux-gnu".to_string(),
            TargetArtifact {
                url: "https://example.com/p.zip".to_string(),
                sha256: "abc".to_string(),
                executable_path: "p".to_string(),
            },
        );
        Package {
            id: ScopedId::new("owner", "pkg"),
            description: "desc".to_string(),
            repo: "https://example.com".to_string(),
            package_type: PackageType::Plugin,
            tags: vec![],
            versions: vec![VersionEntry {
                version: semver::Version::new(1, 0, 0),
                nu_version: nu_constraint.to_string(),
                verified_with: vec![],
                artifact: Artifact {
                    kind: "binary".to_string(),
                    url: None,
                    sha256: None,
                    targets,
                    archive_root: None,
                    include: None,
                    entry: None,
                },
                source: None,
                dependencies: BTreeMap::new(),
                activation: None,
            }],
        }
    }

    #[test]
    fn has_compatible_respects_nu_constraint() {
        let platform = Platform {
            os: crate::core::platform::Os::Linux,
            arch: crate::core::platform::Arch::X86_64,
            env: crate::core::platform::Env::Gnu,
            triple: "x86_64-unknown-linux-gnu".to_string(),
        };
        let nu = NuVersion::parse("0.114.1").unwrap();
        let resolver = Resolver::new(&platform, &nu);
        assert!(!resolver.has_compatible_version(&sample_pkg(">=0.113.0 <0.114.0")));
        assert!(resolver.has_compatible_version(&sample_pkg(">=0.114.0")));
    }
}
