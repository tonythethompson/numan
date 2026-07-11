use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::core::integrity;
use crate::core::platform::Platform;
use crate::nu::bootstrap::managed_nu_binary;
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

    /// Nu data directory (`$nu.data-dir`), e.g. `~/.local/share/nushell` on Linux
    /// or `%APPDATA%\nushell` on Windows.
    #[serde(default)]
    pub data_dir: Option<String>,

    /// All vendor-autoload directories reported by Nu (`$nu.vendor-autoload-dirs`).
    #[serde(default)]
    pub vendor_autoload_dirs: Vec<String>,

    /// The selected vendor-autoload target for Numan.
    ///
    /// Set only when `<$nu.data-dir>/vendor/autoload` is present in
    /// `$nu.vendor-autoload-dirs`. `None` means no safe target is available.
    #[serde(default)]
    pub vendor_autoload_dir: Option<String>,
}

/// Structured output from the Nu probe program.
#[derive(Debug, Deserialize)]
struct NuProbeOutput {
    version: String,
    plugin_path: String,
    data_dir: String,
    vendor_autoload_dirs: Vec<String>,
}

/// Nu probe program — single invocation, emits one JSON object containing
/// version, plugin-path, data-dir, and vendor-autoload-dirs.
///
/// Using a JSON object avoids brittle line-splitting and handles paths that
/// contain newlines or other unusual characters correctly.
const PROBE_SCRIPT: &str = r#"{
  version: (version | get version),
  plugin_path: $nu.plugin-path,
  data_dir: $nu.data-dir,
  vendor_autoload_dirs: $nu.vendor-autoload-dirs
} | to json"#;

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
        let probe = probe_nu(&nu_exe)?;
        let nu_bytes = std::fs::read(&nu_exe)
            .with_context(|| format!("Failed to read Nu binary at '{nu_exe}'"))?;
        let nu_hash = integrity::compute_sha256(&nu_bytes);
        let platform = Platform::detect();

        // Select the safe vendor-autoload target: <$nu.data-dir>/vendor/autoload,
        // but only when it is present in Nu's reported vendor-autoload-dirs list.
        let vendor_autoload_dir =
            select_vendor_autoload_dir(&probe.data_dir, &probe.vendor_autoload_dirs)?;

        Ok(Self {
            nu_executable: nu_exe,
            nu_version: probe.version,
            plugin_registry_path: probe.plugin_path,
            nu_executable_hash: nu_hash,
            platform: platform.triple.clone(),
            data_dir: Some(probe.data_dir),
            vendor_autoload_dirs: probe.vendor_autoload_dirs,
            vendor_autoload_dir,
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

    /// Validate that the vendor-autoload environment has not drifted since init.
    ///
    /// Returns `Err` when:
    /// - `data_dir` was never cached (old init, needs refresh)
    /// - `vendor_autoload_dirs` differs from the current probe (needs refresh)
    /// - the previously selected target is no longer in the reported list
    pub fn validate_vendor_drift(&self, probe_dirs: &[String]) -> Result<()> {
        if self.data_dir.is_none() {
            bail!("Nu data directory not cached. Run 'numan init --refresh' to update.");
        }

        // Normalize and sort both sides — Nu may return dirs in different order across runs.
        let mut cached: Vec<PathBuf> = self
            .vendor_autoload_dirs
            .iter()
            .map(|s| normalize_path(Path::new(s)))
            .collect();
        cached.sort();
        let mut current: Vec<PathBuf> = probe_dirs
            .iter()
            .map(|s| normalize_path(Path::new(s)))
            .collect();
        current.sort();

        if cached != current {
            bail!(
                "Nu vendor-autoload directories have changed since init. \
                 Run 'numan init --refresh'."
            );
        }

        // If a target was selected, verify it is still in the list.
        if let Some(selected) = &self.vendor_autoload_dir {
            let selected_norm = normalize_path(Path::new(selected));
            if !current.contains(&selected_norm) {
                bail!(
                    "Previously selected vendor-autoload directory '{}' is no longer \
                     in $nu.vendor-autoload-dirs. Run 'numan init --refresh'.",
                    selected
                );
            }
        }

        Ok(())
    }
}

/// Locate the `nu` executable on PATH, then under the Numan-managed tools directory.
pub fn find_nu_executable() -> Result<String> {
    find_nu_executable_with_root(&Config::resolve_root(&Platform::detect()))
}

/// Locate `nu` on PATH, then under `<root>/tools/nushell/`.
pub fn find_nu_executable_with_root(root: &Path) -> Result<String> {
    if let Ok(path) = find_nu_on_path() {
        return Ok(path);
    }

    let managed = managed_nu_binary(root);
    if managed.is_file() {
        return Ok(managed.to_string_lossy().into_owned());
    }

    bail!(
        "Nu not found on PATH or in '{}'. Install Nushell with: numan setup nu",
        managed.display()
    );
}

/// Search PATH only (no Numan-managed fallback).
pub fn find_nu_on_path() -> Result<String> {
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

/// Probe `$nu.config-path` from a live Nu binary.
pub fn probe_nu_config_path(nu_exe: &str) -> Result<PathBuf> {
    const PROBE_CONFIG_SCRIPT: &str = r#"{ config_path: $nu.config-path } | to json"#;

    let output = std::process::Command::new(nu_exe)
        .args(["-c", PROBE_CONFIG_SCRIPT])
        .output()
        .with_context(|| format!("Failed to invoke Nu at '{nu_exe}'"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Nu config-path probe failed at '{nu_exe}': {stderr}");
    }

    let stdout = String::from_utf8(output.stdout).context("Nu config-path probe is not UTF-8")?;
    #[derive(Deserialize)]
    struct ConfigProbe {
        config_path: String,
    }
    let probe: ConfigProbe =
        serde_json::from_str(stdout.trim()).context("Nu config-path probe JSON parse failed")?;

    if probe.config_path.is_empty() || probe.config_path == "null" {
        bail!("Nu config-path probe returned an empty config path.");
    }

    Ok(PathBuf::from(probe.config_path))
}

/// Run a single Nu invocation and parse the resulting JSON probe output.
///
/// The probe emits one JSON object, so no ad-hoc line splitting is needed.
fn probe_nu(nu_exe: &str) -> Result<NuProbeOutput> {
    let output = std::process::Command::new(nu_exe)
        .args(["-c", PROBE_SCRIPT])
        .output()
        .with_context(|| format!("Failed to invoke Nu at '{nu_exe}'"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Nu probe failed at '{nu_exe}': {stderr}");
    }

    let stdout = String::from_utf8(output.stdout).context("Nu probe output is not UTF-8")?;
    let probe: NuProbeOutput =
        serde_json::from_str(stdout.trim()).context("Nu probe JSON parse failed")?;

    if probe.plugin_path.is_empty() || probe.plugin_path == "null" {
        bail!(
            "Nu probe returned empty plugin-path. \
             Ensure Nu is configured with a plugin registry."
        );
    }

    Ok(probe)
}

/// Normalize a filesystem path for comparison: canonicalize if it exists,
/// otherwise use the lexically-normalized form. On Windows, strip the
/// extended-length prefix (`\\?\`) before comparing.
fn normalize_path(path: &Path) -> PathBuf {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

    // Strip Windows extended-length path prefix so comparisons are uniform.
    #[cfg(target_os = "windows")]
    {
        let s = canonical.to_string_lossy();
        if let Some(stripped) = s.strip_prefix(r"\\?\") {
            return PathBuf::from(stripped);
        }
    }

    canonical
}

/// Select the Numan-safe vendor-autoload directory.
///
/// The safe target is `<data_dir>/vendor/autoload`. It is returned only when
/// it is present in Nu's reported `vendor_autoload_dirs` list (after path
/// normalization). If absent, returns `None` — the caller decides whether to
/// error or warn.
fn select_vendor_autoload_dir(
    data_dir: &str,
    vendor_autoload_dirs: &[String],
) -> Result<Option<String>> {
    let expected: PathBuf = Path::new(data_dir).join("vendor").join("autoload");
    let expected_norm = normalize_path(&expected);

    let found = vendor_autoload_dirs
        .iter()
        .find(|d| normalize_path(Path::new(d.as_str())) == expected_norm);

    match found {
        Some(dir) => Ok(Some(dir.clone())),
        None => {
            // Not an error at detection time — `detect()` caches `None`.
            // Commands that need module activation will report a clear error.
            Ok(None)
        }
    }
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
            data_dir: None,
            vendor_autoload_dirs: vec![],
            vendor_autoload_dir: None,
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
        assert_eq!(loaded.data_dir, None);
        assert!(loaded.vendor_autoload_dirs.is_empty());
        assert_eq!(loaded.vendor_autoload_dir, None);
    }

    #[test]
    fn paths_roundtrip_with_vendor_fields() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let mut paths = fake_nu_paths("/usr/bin/nu", "abc123");
        paths.data_dir = Some("/home/user/.local/share/nushell".to_string());
        paths.vendor_autoload_dirs = vec![
            "/home/user/.local/share/nushell/vendor/autoload".to_string(),
            "/usr/share/nushell/vendor/autoload".to_string(),
        ];
        paths.vendor_autoload_dir =
            Some("/home/user/.local/share/nushell/vendor/autoload".to_string());
        paths.save(&root).unwrap();
        let loaded = NuPaths::load(&root).unwrap();
        assert_eq!(
            loaded.data_dir.as_deref(),
            Some("/home/user/.local/share/nushell")
        );
        assert_eq!(loaded.vendor_autoload_dirs.len(), 2);
        assert_eq!(
            loaded.vendor_autoload_dir.as_deref(),
            Some("/home/user/.local/share/nushell/vendor/autoload")
        );
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
    fn probe_script_emits_json() {
        // Verify the probe script constant contains the key JSON fields.
        assert!(PROBE_SCRIPT.contains("version"));
        assert!(PROBE_SCRIPT.contains("plugin_path"));
        assert!(PROBE_SCRIPT.contains("data_dir"));
        assert!(PROBE_SCRIPT.contains("vendor_autoload_dirs"));
        assert!(PROBE_SCRIPT.contains("to json"));
    }

    // ── vendor target selection ───────────────────────────────────────────────

    #[test]
    fn vendor_target_selected_when_present() {
        let data_dir = "/home/user/.local/share/nushell";
        let expected = "/home/user/.local/share/nushell/vendor/autoload";
        let dirs = vec![
            "/usr/share/nushell/vendor/autoload".to_string(),
            expected.to_string(),
        ];
        let result = select_vendor_autoload_dir(data_dir, &dirs).unwrap();
        assert_eq!(result.as_deref(), Some(expected));
    }

    #[test]
    fn vendor_target_none_when_absent() {
        let data_dir = "/home/user/.local/share/nushell";
        let dirs = vec![
            "/usr/share/nushell/vendor/autoload".to_string(),
            "/etc/nushell/vendor/autoload".to_string(),
        ];
        let result = select_vendor_autoload_dir(data_dir, &dirs).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn vendor_target_none_for_empty_list() {
        let result = select_vendor_autoload_dir("/home/user/.local/share/nushell", &[]).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn vendor_drift_detected_when_dirs_change() {
        let mut paths = fake_nu_paths("/usr/bin/nu", "abc123");
        paths.data_dir = Some("/home/user/.local/share/nushell".to_string());
        paths.vendor_autoload_dirs =
            vec!["/home/user/.local/share/nushell/vendor/autoload".to_string()];

        // Same dirs — no drift.
        paths
            .validate_vendor_drift(&["/home/user/.local/share/nushell/vendor/autoload".to_string()])
            .unwrap();

        // Different dirs — drift.
        let err = paths
            .validate_vendor_drift(&["/different/path/vendor/autoload".to_string()])
            .unwrap_err();
        assert!(
            err.to_string().contains("changed") || err.to_string().contains("vendor-autoload"),
            "Expected vendor drift error, got: {err}"
        );
    }

    #[test]
    fn vendor_drift_detected_when_data_dir_not_cached() {
        let paths = fake_nu_paths("/usr/bin/nu", "abc123");
        // data_dir is None — requires refresh.
        let err = paths.validate_vendor_drift(&[]).unwrap_err();
        assert!(
            err.to_string().contains("data directory") || err.to_string().contains("refresh"),
            "Expected missing data_dir error, got: {err}"
        );
    }

    // ── backward compatibility: old JSON without vendor fields round-trips ───

    #[test]
    fn old_lockfile_without_vendor_fields_loads_with_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::create_dir_all(root.join("nu_state")).unwrap();
        // Write a paths.json that does NOT include the new Phase 4 fields.
        let old_json = r#"{
            "nu_executable": "/usr/bin/nu",
            "nu_version": "0.112.0",
            "plugin_registry_path": "/home/user/.config/nushell/plugin.msgpackz",
            "nu_executable_hash": "deadbeef",
            "platform": "x86_64-unknown-linux-gnu"
        }"#;
        std::fs::write(root.join("nu_state/paths.json"), old_json).unwrap();
        let loaded = NuPaths::load(&root).unwrap();
        assert_eq!(loaded.data_dir, None);
        assert!(loaded.vendor_autoload_dirs.is_empty());
        assert_eq!(loaded.vendor_autoload_dir, None);
    }
}
