use anyhow::{bail, Context, Result};

use crate::core::nu_version::NuVersion;
use crate::core::package::{Package, PackageType, VersionEntry};
use crate::core::platform::Platform;

pub struct Resolver<'a> {
    pub platform: &'a Platform,
    pub nu_version: &'a NuVersion,
}

impl<'a> Resolver<'a> {
    pub fn new(platform: &'a Platform, nu_version: &'a NuVersion) -> Self {
        Self {
            platform,
            nu_version,
        }
    }

    pub fn resolve<'b>(&self, package: &'b Package) -> Result<&'b VersionEntry> {
        let mut candidates: Vec<&VersionEntry> = package
            .versions
            .iter()
            .filter(|v| self.is_compatible(v))
            .collect();

        if candidates.is_empty() {
            let available: Vec<String> = package
                .versions
                .iter()
                .map(|v| v.version.to_string())
                .collect();

            match package.package_type {
                PackageType::Plugin => {
                    bail!(
                        "No compatible version found for '{}'.
       Your Nu version: {} ({})
       Available versions: {}
       Options:
         - Upgrade Nu: https://www.nushell.sh/book/installation.html
         - Install an older version: numan install {}@<version>",
                        package.id,
                        self.nu_version.version,
                        self.platform.triple,
                        available.join(", "),
                        package.id
                    );
                }
                _ => {
                    bail!(
                        "No compatible version found for '{}' on {}.",
                        package.id,
                        self.platform.triple
                    );
                }
            }
        }

        // Sort by version descending, return latest
        candidates.sort_by(|a, b| b.version.cmp(&a.version));
        Ok(candidates[0])
    }

    pub fn is_compatible(&self, version: &VersionEntry) -> bool {
        // Check Nu version constraint
        if !self.nu_version.matches_constraint(&version.nu_version) {
            return false;
        }

        // For plugins, check that a binary exists for our target
        if version.artifact.kind == "binary" {
            return version.artifact.targets.contains_key(&self.platform.triple);
        }

        // For modules/scripts/completions, just need an artifact
        true
    }

    /// Resolve an exact version with compatibility validation.
    /// Returns an error if the version exists but is not compatible.
    pub fn resolve_exact<'b>(
        &self,
        package: &'b Package,
        target_version: &semver::Version,
    ) -> Result<&'b VersionEntry> {
        let entry = package
            .versions
            .iter()
            .find(|v| v.version == *target_version)
            .with_context(|| {
                let available: Vec<String> = package
                    .versions
                    .iter()
                    .map(|v| v.version.to_string())
                    .collect();
                format!(
                    "Version {target_version} not available for '{}'. Available: {}",
                    package.id,
                    available.join(", ")
                )
            })?;

        // Validate compatibility even for explicit versions
        if !self.is_compatible(entry) {
            let mut reasons = Vec::new();
            if !self.nu_version.matches_constraint(&entry.nu_version) {
                reasons.push(format!(
                    "Nu version {} does not satisfy constraint '{}'",
                    self.nu_version.version, entry.nu_version
                ));
            }
            if entry.artifact.kind == "binary"
                && !entry.artifact.targets.contains_key(&self.platform.triple)
            {
                reasons.push(format!(
                    "No binary for target '{}'",
                    self.platform.triple
                ));
            }
            bail!(
                "Version {target_version} of '{}' is not compatible with your system: {}",
                package.id,
                reasons.join("; ")
            );
        }

        Ok(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::package::*;
    use std::collections::HashMap;

    fn test_plugin() -> Package {
        let mut targets = HashMap::new();
        targets.insert(
            "x86_64-pc-windows-msvc".to_string(),
            TargetArtifact {
                url: "https://example.com/win.zip".to_string(),
                sha256: "abc".to_string(),
                executable_path: "nu_plugin_test.exe".to_string(),
            },
        );
        targets.insert(
            "x86_64-unknown-linux-gnu".to_string(),
            TargetArtifact {
                url: "https://example.com/linux.tar.gz".to_string(),
                sha256: "def".to_string(),
                executable_path: "nu_plugin_test".to_string(),
            },
        );

        Package {
            id: ScopedId::new("test", "plugin"),
            description: "Test plugin".to_string(),
            repo: "https://github.com/test/plugin".to_string(),
            package_type: PackageType::Plugin,
            tags: vec![],
            versions: vec![
                VersionEntry {
                    version: semver::Version::new(2, 0, 0),
                    nu_version: ">=0.113.0 <0.114.0".to_string(),
                    verified_with: vec![],
                    artifact: Artifact {
                        kind: "binary".to_string(),
                        url: None,
                        sha256: None,
                        targets: targets.clone(),
                        archive_root: None,
                        include: None,
                        entry: None,
                    },
                    source: None,
                    dependencies: HashMap::new(),
                },
                VersionEntry {
                    version: semver::Version::new(1, 0, 0),
                    nu_version: ">=0.112.0 <0.113.0".to_string(),
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
                    dependencies: HashMap::new(),
                },
            ],
        }
    }

    fn linux_platform() -> Platform {
        Platform {
            os: crate::core::platform::Os::Linux,
            arch: crate::core::platform::Arch::X86_64,
            env: crate::core::platform::Env::Gnu,
            triple: "x86_64-unknown-linux-gnu".to_string(),
        }
    }

    #[test]
    fn resolve_latest_compatible_plugin() {
        let platform = linux_platform();
        let nu = NuVersion::parse("0.113.1").unwrap();
        let resolver = Resolver::new(&platform, &nu);
        let pkg = test_plugin();
        let resolved = resolver.resolve(&pkg).unwrap();
        assert_eq!(resolved.version, semver::Version::new(2, 0, 0));
    }

    #[test]
    fn resolve_falls_back_to_older_version() {
        let platform = linux_platform();
        let nu = NuVersion::parse("0.112.5").unwrap();
        let resolver = Resolver::new(&platform, &nu);
        let pkg = test_plugin();
        let resolved = resolver.resolve(&pkg).unwrap();
        assert_eq!(resolved.version, semver::Version::new(1, 0, 0));
    }

    #[test]
    fn resolve_no_compatible_version() {
        let platform = linux_platform();
        let nu = NuVersion::parse("0.110.0").unwrap();
        let resolver = Resolver::new(&platform, &nu);
        let pkg = test_plugin();
        assert!(resolver.resolve(&pkg).is_err());
    }

    #[test]
    fn resolve_exact_compatible() {
        let platform = linux_platform();
        let nu = NuVersion::parse("0.113.1").unwrap();
        let resolver = Resolver::new(&platform, &nu);
        let pkg = test_plugin();
        let v = semver::Version::new(2, 0, 0);
        let resolved = resolver.resolve_exact(&pkg, &v).unwrap();
        assert_eq!(resolved.version, v);
    }

    #[test]
    fn resolve_exact_incompatible_nu() {
        let platform = linux_platform();
        let nu = NuVersion::parse("0.112.5").unwrap();
        let resolver = Resolver::new(&platform, &nu);
        let pkg = test_plugin();
        // v2.0.0 requires >=0.113.0, but we have 0.112.5
        let v = semver::Version::new(2, 0, 0);
        let result = resolver.resolve_exact(&pkg, &v);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not compatible"));
    }

    #[test]
    fn resolve_exact_incompatible_target() {
        let platform = linux_platform(); // linux
        let nu = NuVersion::parse("0.112.5").unwrap();
        let resolver = Resolver::new(&platform, &nu);
        let pkg = test_plugin();
        // v1.0.0 only has linux target, but let's test if we can construct a version without it
        // Actually v1.0.0 has both targets. Let's test with a missing target.
        // We'll test this with a different package.
        let mut pkg_no_target = pkg.clone();
        pkg_no_target.versions[1].artifact.targets.clear(); // v1.0.0 now has no targets
        let v = semver::Version::new(1, 0, 0);
        let result = resolver.resolve_exact(&pkg_no_target, &v);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("No binary"));
    }

    #[test]
    fn resolve_exact_not_found() {
        let platform = linux_platform();
        let nu = NuVersion::parse("0.113.1").unwrap();
        let resolver = Resolver::new(&platform, &nu);
        let pkg = test_plugin();
        let v = semver::Version::new(99, 0, 0);
        assert!(resolver.resolve_exact(&pkg, &v).is_err());
    }
}
