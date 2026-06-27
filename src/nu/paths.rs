use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NuPaths {
    pub nu_executable: String,
    pub nu_version: String,
    pub plugin_registry_path: Option<String>,
    pub vendor_autoload_dir: Option<String>,
    pub config_dir: Option<String>,
    pub data_dir: Option<String>,
    pub fingerprint: String,
}

impl NuPaths {
    pub fn load(root: &PathBuf) -> Result<Self> {
        let paths_path = root.join("nu_state/paths.json");
        if !paths_path.exists() {
            anyhow::bail!(
                "Numan not initialized. Run 'numan init' first."
            );
        }
        let content = std::fs::read_to_string(&paths_path)
            .with_context(|| format!("Failed to read {}", paths_path.display()))?;
        let paths: NuPaths = serde_json::from_str(&content)?;
        Ok(paths)
    }

    pub fn save(&self, root: &PathBuf) -> Result<()> {
        let paths_path = root.join("nu_state/paths.json");
        if let Some(parent) = paths_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(paths_path, content)?;
        Ok(())
    }

    pub fn detect(_root: &PathBuf) -> Result<Self> {
        let output = std::process::Command::new("nu")
            .arg("--version")
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to run 'nu --version': {e}"))?;

        let _version = String::from_utf8(output.stdout)
            .unwrap_or_default()
            .trim()
            .to_string();

        let version_output = std::process::Command::new("nu")
            .arg("--version")
            .output();

        let version = match version_output {
            Ok(o) => String::from_utf8(o.stdout).unwrap_or_default().trim().to_string(),
            Err(_) => "unknown".to_string(),
        };

        // Try to find the plugin registry path
        let plugin_path_output = std::process::Command::new("nu")
            .arg("--commands")
            .arg("echo $nu.plugin-path")
            .output();

        let plugin_registry = match plugin_path_output {
            Ok(o) => {
                let path = String::from_utf8(o.stdout).unwrap_or_default().trim().to_string();
                if !path.is_empty() && path != "null" {
                    Some(path)
                } else {
                    None
                }
            }
            Err(_) => None,
        };

        let fingerprint = format!(
            "{:016}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );

        Ok(Self {
            nu_executable: "nu".to_string(),
            nu_version: version,
            plugin_registry_path: plugin_registry,
            vendor_autoload_dir: None,
            config_dir: None,
            data_dir: None,
            fingerprint,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_roundtrip() {
        let paths = NuPaths {
            nu_executable: "nu".to_string(),
            nu_version: "0.113.1".to_string(),
            plugin_registry_path: Some("/path/to/plugins.nu".to_string()),
            vendor_autoload_dir: None,
            config_dir: None,
            data_dir: None,
            fingerprint: "1234567890123456".to_string(),
        };

        let json = serde_json::to_string_pretty(&paths).unwrap();
        let parsed: NuPaths = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.nu_version, "0.113.1");
    }
}
