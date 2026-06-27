use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::core::integrity;
use crate::core::platform::Platform;
use crate::util::atomic::write_json_atomic;

/// Nu environment state, cached to `<root>/nu_state/paths.json` at `numan init`.
///
/// All fields are absolute paths resolved at init time. `validate_drift` must
/// be called before any command that invokes Nu, to detect binary replacement
/// or Nu upgrades since init.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NuPaths {
    /// Absolute path to the Nu binary (e.g. "/usr/bin/nu" or "C:\…\nu.exe").
    pub nu_executable: String,
    /// Nu version string (e.g. "0.113.1").
    pub nu_version: String,
    /// Absolute path to the plugin registry file.
    pub plugin_registry_path: String,
    /// SHA256 of the Nu binary at init time — used for drift detection.
    pub nu_executable_hash: String,
    /// Platform triple at init time.
    pub platform: String,
    // Deferred to later phases:
    pub vendor_autoload_dir: Option<String>,
    pub config_dir: Option<String>,
    pub data_dir: Option<String>,
}

/// Nu probe program — single invocation, two output lines:
///   line 1: version string (e.g. "0.113.1")
///   line 2: absolute plugin-registry path
const PROBE_SCRIPT: &str = "print (version | get version); print $nu.plugin-path";

impl NuPaths {
    pub fn load(root: &Path) -> Result<Self> {
        let paths_path = root.join("nu_state/paths.json");
        if !paths_path.exists() {
            bail!("Numan not initialized. Run 'numan init' first.");
        }
        let content = std::fs::read_to_string(&paths_path)
            .with_context(|| format!("Failed to read {}", paths_path.display()))?;
        let paths: NuPaths = serde_json::from_str(&content)?;
        Ok(paths)
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let paths_path = root.join("nu_state/paths.json");
        write_json_atomic(&paths_path, self)
    }

    /// Discover Nu on PATH, probe it once, and build a `NuPaths`.
    ///
    /// Called only by `numan init` / `numan init --refresh`. The `activate`
    /// command calls `load()` then `validate_drift()` — never `detect()`.
    pub fn detect() -> Result<Self> {
        let nu_exe = find_nu_executable()?;
        let (nu_version, plugin_registry_path) = probe_nu(&nu_exe)?;
        let nu_bytes = std::fs::read(&nu_exe)
            .with_context(|| format!("Failed to read Nu binary at '{nu_exe}'"))?;
        let nu_hash = integrity::compute_sha256(&nu_bytes);
        let platform = Platform::detect();

        Ok(Self {
            nu_executable: nu_exe,
            nu_version,
            plugin_registry_path,
            nu_executable_hash: nu_hash,
            platform: platform.triple.clone(),
            vendor_autoload_dir: None,
            config_dir: None,
            data_dir: None,
        })
    }

    /// Verify that the cached Nu binary still exists, its SHA256 still matches,
    /// and the plugin registry parent directory still exists.
    ///
    /// Returns `Err` with a `numan init --refresh` hint on any mismatch.
    pub fn validate_drift(&self) -> Result<()> {
        let exe_path = Path::new(&self.nu_executable);
        if !exe_path.exists() {
            bail!(
                "Nu binary not found at '{}'. Run 'numan init --refresh'.",
                self.nu_executable
            );
        }

        let bytes = std::fs::read(exe_path)
            .with_context(|| format!("Failed to read Nu binary at '{}'", self.nu_executable))?;
        let current_hash = integrity::compute_sha256(&bytes);
        if current_hash != self.nu_executable_hash {
            bail!(
                "Nu binary has changed since init (hash mismatch at '{}'). \
                 Run 'numan init --refresh'.",
                self.nu_executable
            );
        }

        let registry = Path::new(&self.plugin_registry_path);
        if !registry.is_absolute() {
            bail!(
                "Cached plugin registry path is not absolute: '{}'. \
                 Run 'numan init --refresh'.",
                self.plugin_registry_path
            );
        }
        if let Some(parent) = registry.parent() {
            if !parent.exists() {
                bail!(
                    "Plugin registry parent directory does not exist: '{}'. \
                     Run 'numan init --refresh'.",
                    parent.display()
                );
            }
        }

        Ok(())
    }
}

/// Locate the `nu` executable by searching PATH via platform-native tool.
/// Only called at `numan init` time.
fn find_nu_executable() -> Result<String> {
    #[cfg(target_os = "windows")]
    {
        let output = std::process::Command::new("where.exe")
            .arg("nu")
            .output()
            .context("Failed to run 'where.exe nu' — is Nushell on PATH?")?;
        if !output.status.success() {
            bail!("Nu not found on PATH. Is Nushell installed?");
        }
        let stdout = String::from_utf8(output.stdout).context("where.exe output is not UTF-8")?;
        let path = stdout
            .lines()
            .next()
            .ok_or_else(|| anyhow::anyhow!("where.exe returned no results for 'nu'"))?
            .trim()
            .to_string();
        if path.is_empty() {
            bail!("Nu not found on PATH. Is Nushell installed?");
        }
        Ok(path)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let output = std::process::Command::new("which")
            .arg("nu")
            .output()
            .context("Failed to run 'which nu' — is Nushell on PATH?")?;
        if !output.status.success() {
            bail!("Nu not found on PATH. Is Nushell installed?");
        }
        let path = String::from_utf8(output.stdout)
            .context("which output is not UTF-8")?
            .trim()
            .to_string();
        if path.is_empty() {
            bail!("Nu not found on PATH. Is Nushell installed?");
        }
        Ok(path)
    }
}

/// Run a single Nu invocation to get version + plugin-registry-path.
fn probe_nu(nu_exe: &str) -> Result<(String, String)> {
    let output = std::process::Command::new(nu_exe)
        .args(["-c", PROBE_SCRIPT])
        .output()
        .with_context(|| format!("Failed to invoke Nu at '{nu_exe}'"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Nu probe failed at '{nu_exe}': {stderr}");
    }

    let stdout = String::from_utf8(output.stdout).context("Nu probe output is not UTF-8")?;
    let mut lines = stdout.lines();

    let version = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("Nu probe: expected version on line 1, got no output"))?
        .trim()
        .to_string();

    let plugin_path = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("Nu probe: expected plugin-path on line 2"))?
        .trim()
        .to_string();

    if plugin_path.is_empty() || plugin_path == "null" {
        bail!(
            "Nu probe returned empty plugin-path. \
             Ensure Nu is configured with a plugin registry."
        );
    }

    Ok((version, plugin_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_nu_paths(nu_exe: &str, nu_hash: &str) -> NuPaths {
        NuPaths {
            nu_executable: nu_exe.to_string(),
            nu_version: "0.113.1".to_string(),
            plugin_registry_path: "/path/to/plugins.msgpackz".to_string(),
            nu_executable_hash: nu_hash.to_string(),
            platform: "x86_64-pc-windows-msvc".to_string(),
            vendor_autoload_dir: None,
            config_dir: None,
            data_dir: None,
        }
    }

    #[test]
    fn paths_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let original = fake_nu_paths("/usr/bin/nu", "abc123");
        original.save(&root).unwrap();
        let loaded = NuPaths::load(&root).unwrap();
        assert_eq!(loaded.nu_version, "0.113.1");
        assert_eq!(loaded.nu_executable_hash, "abc123");
        assert_eq!(loaded.plugin_registry_path, "/path/to/plugins.msgpackz");
    }

    #[test]
    fn load_errors_when_not_initialized() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let err = NuPaths::load(&root).unwrap_err();
        assert!(err.to_string().contains("numan init"));
    }

    #[test]
    fn nu_paths_drift_detection() {
        let dir = tempfile::tempdir().unwrap();

        // Create a fake nu binary
        let fake_nu = dir.path().join("nu");
        std::fs::write(&fake_nu, b"fake nu binary v1").unwrap();
        let hash_v1 = integrity::compute_sha256(b"fake nu binary v1");

        // Create a registry dir so parent exists check passes
        let reg_dir = dir.path().join("nushell");
        std::fs::create_dir_all(&reg_dir).unwrap();
        let reg_path = reg_dir.join("plugin.msgpackz");

        let mut paths = fake_nu_paths(&fake_nu.to_string_lossy(), &hash_v1);
        paths.plugin_registry_path = reg_path.to_string_lossy().to_string();

        // Should pass with correct hash
        paths.validate_drift().unwrap();

        // Overwrite binary — hash changes
        std::fs::write(&fake_nu, b"fake nu binary v2 (updated)").unwrap();
        let err = paths.validate_drift().unwrap_err();
        assert!(
            err.to_string().contains("hash mismatch") || err.to_string().contains("changed"),
            "Expected drift error, got: {err}"
        );
    }

    #[test]
    fn nu_paths_validate_missing_executable() {
        let dir = tempfile::tempdir().unwrap();
        let paths = fake_nu_paths(
            &dir.path().join("nonexistent_nu").to_string_lossy(),
            "abc123",
        );
        let err = paths.validate_drift().unwrap_err();
        assert!(err.to_string().contains("not found") || err.to_string().contains("numan init"));
    }

    #[test]
    fn nu_paths_validate_non_absolute_registry() {
        let dir = tempfile::tempdir().unwrap();
        let fake_nu = dir.path().join("nu");
        std::fs::write(&fake_nu, b"fake").unwrap();
        let hash = integrity::compute_sha256(b"fake");

        let mut paths = fake_nu_paths(&fake_nu.to_string_lossy(), &hash);
        paths.plugin_registry_path = "relative/path.msgpackz".to_string();

        let err = paths.validate_drift().unwrap_err();
        assert!(err.to_string().contains("not absolute") || err.to_string().contains("numan init"));
    }

    #[test]
    fn probe_script_is_static() {
        // Verify the probe script constant is what we expect
        assert!(PROBE_SCRIPT.contains("version"));
        assert!(PROBE_SCRIPT.contains("plugin-path"));
        assert!(PROBE_SCRIPT.contains("print"));
    }
}
