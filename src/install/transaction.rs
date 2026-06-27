use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use crate::core::integrity;
use crate::core::nu_version::NuVersion;
use crate::core::package::{PackageType, ScopedId};
use crate::core::platform::Platform;
use crate::core::registry::RegistryManager;
use crate::core::resolve::Resolver;
use crate::install::download;
use crate::install::extract;
use crate::install::extract::ExtractConfig;
use crate::state::lockfile::{Lockfile, LockfileEntry};

pub struct InstallOptions<'a> {
    pub root: &'a PathBuf,
    pub platform: &'a Platform,
    pub nu_version: &'a NuVersion,
    pub force: bool,
    pub verbose: bool,
}

pub struct InstallResult {
    pub installed: bool,
    pub package: String,
    pub version: String,
    pub path: PathBuf,
    pub already_existed: bool,
}

pub fn install_package(
    package_id: &str,
    version: Option<&str>,
    options: &InstallOptions,
) -> Result<InstallResult> {
    // 1. Parse package ID
    let id = ScopedId::parse(package_id)?;

    // 2. Load registry
    let registry = RegistryManager::new(options.root)?;

    // 3. Find package in index
    let pkg = registry
        .find_package(&id.to_string())?
        .with_context(|| format!("Package '{}' not found in registry", id))?;

    // 4. Resolve version
    let resolver = Resolver::new(options.platform, options.nu_version);
    let resolved = if let Some(ver_str) = version {
        // Parse exact version request
        let target_version: semver::Version = ver_str
            .parse()
            .with_context(|| format!("Invalid version: '{ver_str}'"))?;

        pkg.versions
            .iter()
            .find(|v| v.version == target_version)
            .with_context(|| format!(
                "Version {ver_str} not available for '{}'",
                id
            ))?
    } else {
        resolver.resolve(&pkg)?
    };

    let version_str = resolved.version.to_string();

    // 5. Determine artifact
    let (artifact_url, artifact_sha256, executable_path) = if resolved.artifact.kind == "binary" {
        // Plugin: get platform-specific artifact
        let target = resolved
            .artifact
            .targets
            .get(&options.platform.triple)
            .with_context(|| format!(
                "No binary available for '{}' on {}",
                id, options.platform.triple
            ))?;

        (
            target.url.clone(),
            Some(target.sha256.clone()),
            Some(target.executable_path.clone()),
        )
    } else {
        // Module/script/completion: use artifact.url or target
        let url = resolved
            .artifact
            .url
            .clone()
            .or_else(|| {
                resolved
                    .artifact
                    .targets
                    .values()
                    .next()
                    .map(|t| t.url.clone())
            })
            .with_context(|| format!("No artifact URL for '{}'", id))?;

        (url, resolved.artifact.sha256.clone(), None)
    };

    // 6. Check if already installed
    let mut lockfile = Lockfile::load(options.root)?;
    let lock_key = id.to_string();

    if !options.force {
        if let Some(entry) = lockfile.packages.get(&lock_key) {
            if entry.version == version_str {
                let pkg_dir = compute_install_path(
                    options.root,
                    &pkg.package_type,
                    &id,
                    &version_str,
                    &artifact_url,
                );

                if pkg_dir.exists() {
                    if options.verbose {
                        println!(
                            "{} {}@{} is already installed",
                            console::style("✓").green(),
                            id,
                            version_str
                        );
                    }
                    return Ok(InstallResult {
                        installed: false,
                        package: lock_key,
                        version: version_str,
                        path: pkg_dir,
                        already_existed: true,
                    });
                }
            }
        }
    }

    // 7. Compute immutable install path
    let pkg_type_dir = pkg.package_type.dir_name();
    let short_hash = compute_short_hash(&artifact_url);
    let version_dir = format!("{version_str}-{short_hash}");
    let install_dir = options
        .root
        .join("packages")
        .join(pkg_type_dir)
        .join(&id.owner)
        .join(&id.name)
        .join(&version_dir);

    // 8. Download to cache
    let cache_dir = options.root.join("cache/downloads");
    std::fs::create_dir_all(&cache_dir)?;

    let cache_file = if let Some(ref sha) = artifact_sha256 {
        cache_dir.join(format!("{sha}.bin"))
    } else {
        // Compute hash from URL as filename
        let url_hash = compute_short_hash(&artifact_url);
        cache_dir.join(format!("{url_hash}.bin"))
    };

    if !cache_file.exists() || options.force {
        if options.verbose {
            println!(
                "{} Downloading {}@{}...",
                console::style("↓").cyan(),
                id,
                version_str
            );
        }
        download::download_file(&artifact_url, &cache_file)?;
    } else if options.verbose {
        println!(
            "{} Using cached download",
            console::style("✓").green()
        );
    }

    // 9. Verify SHA256
    if let Some(ref expected_sha) = artifact_sha256 {
        if options.verbose {
            println!("{} Verifying integrity...", console::style("🔒").cyan());
        }
        integrity::verify_and_report(&cache_file, expected_sha, &lock_key)?;
    }

    // 10. Extract to temp, then atomic move
    let tmp_dir = tempfile::tempdir()?;
    let extract_config = ExtractConfig {
        archive_root: resolved.artifact.archive_root.clone(),
        include: resolved.artifact.include.clone(),
        entry: resolved.artifact.entry.clone(),
    };

    if options.verbose {
        println!(
            "{} Extracting to {}...",
            console::style("📦").cyan(),
            install_dir.display()
        );
    }

    let extract_result = extract::extract_archive(&cache_file, tmp_dir.path(), &extract_config)?;

    // Validate executable exists for plugins
    if pkg.package_type == PackageType::Plugin {
        if let Some(ref exe_path) = executable_path {
            let expected_path = tmp_dir.path().join(exe_path);
            if !expected_path.exists() {
                // Try without extension (cross-platform)
                let expected_path_no_ext = tmp_dir.path().join(exe_path.trim_end_matches(".exe"));
                if !expected_path_no_ext.exists() {
                    // Check if any file starts with nu_plugin_
                    let has_plugin = extract_result.files.iter().any(|f| {
                        f.file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| n.starts_with("nu_plugin_"))
                            .unwrap_or(false)
                    });

                    if !has_plugin {
                        bail!(
                            "Expected executable '{}' not found in archive for '{}'",
                            exe_path,
                            id
                        );
                    }
                }
            }
        }
    }

    // Validate entry point if specified
    if let Some(ref entry_name) = resolved.artifact.entry {
        if !extract_result.entry_found {
            bail!(
                "Expected entry point '{}' not found in archive for '{}'",
                entry_name,
                id
            );
        }
    }

    // Atomic move from temp to final location
    if install_dir.exists() {
        std::fs::remove_dir_all(&install_dir)?;
    }
    if let Some(parent) = install_dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(tmp_dir.path(), &install_dir)?;

    // 12. Write lockfile entry
    let installed_at = format_timestamp();
    let entry = LockfileEntry {
        version: version_str.clone(),
        package_type: pkg.package_type.to_string(),
        source: resolved.artifact.kind.clone(),
        target: Some(options.platform.triple.clone()),
        artifact_url: Some(artifact_url.clone()),
        artifact_sha256: artifact_sha256.clone(),
        executable_path,
        archive_root: resolved.artifact.archive_root.clone(),
        include: resolved.artifact.include.clone(),
        entry: resolved.artifact.entry.clone(),
        installed_at,
        nu_version_at_install: Some(options.nu_version.version.clone()),
        activated: false, // Phase 3 will handle activation
        registry_url: None,
        registry_revision: None,
        index_sha256: None,
        signing_key_fingerprint: None,
        git_url: None,
        git_rev: None,
        cargo_name: None,
        cargo_lock_sha256: None,
        built_sha256: None,
    };

    lockfile.packages.insert(lock_key.clone(), entry);
    lockfile.generated_at = format_timestamp();
    lockfile.nu_version = options.nu_version.version.clone();
    lockfile.platform = options.platform.triple.clone();
    lockfile.save(options.root)?;

    // 13. Print success
    println!(
        "{} Installed {}@{} to {}",
        console::style("✓").green(),
        id,
        version_str,
        install_dir.display()
    );

    Ok(InstallResult {
        installed: true,
        package: lock_key,
        version: version_str,
        path: install_dir,
        already_existed: false,
    })
}

/// Compute the install path for a package
fn compute_install_path(
    root: &PathBuf,
    package_type: &PackageType,
    id: &ScopedId,
    version: &str,
    artifact_url: &str,
) -> PathBuf {
    let short_hash = compute_short_hash(artifact_url);
    let version_dir = format!("{version}-{short_hash}");
    root.join("packages")
        .join(package_type.dir_name())
        .join(&id.owner)
        .join(&id.name)
        .join(version_dir)
}

/// Compute first 8 chars of SHA256 of input
fn compute_short_hash(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)[..8].to_string()
}

/// Format current timestamp as ISO 8601
fn format_timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{secs:016}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_short_hash_deterministic() {
        let h1 = compute_short_hash("https://example.com/file.zip");
        let h2 = compute_short_hash("https://example.com/file.zip");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 8);
    }

    #[test]
    fn compute_short_hash_unique() {
        let h1 = compute_short_hash("https://example.com/file1.zip");
        let h2 = compute_short_hash("https://example.com/file2.zip");
        assert_ne!(h1, h2);
    }

    #[test]
    fn compute_install_path_correct() {
        let root = PathBuf::from("/tmp/numan");
        let id = ScopedId::new("test", "plugin");
        let path = compute_install_path(
            &root,
            &PackageType::Plugin,
            &id,
            "1.0.0",
            "https://example.com/file.zip",
        );
        let path_str = path.to_string_lossy();
        assert!(path_str.contains("plugins") && path_str.contains("test") && path_str.contains("plugin"));
        assert!(path_str.contains("1.0.0-"));
    }

    #[test]
    fn format_timestamp_works() {
        let ts = format_timestamp();
        assert_eq!(ts.len(), 16);
        assert!(ts.chars().all(|c| c.is_ascii_digit()));
    }
}
