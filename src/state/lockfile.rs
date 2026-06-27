use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lockfile {
    pub version: u32,
    pub generated_at: String,
    pub nu_version: String,
    pub platform: String,
    pub packages: HashMap<String, LockfileEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockfileEntry {
    pub version: String,
    #[serde(rename = "type")]
    pub package_type: String,
    pub source: String,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub artifact_url: Option<String>,
    #[serde(default)]
    pub artifact_sha256: Option<String>,
    #[serde(default)]
    pub executable_path: Option<String>,
    #[serde(default)]
    pub archive_root: Option<String>,
    #[serde(default)]
    pub include: Option<Vec<String>>,
    #[serde(default)]
    pub entry: Option<String>,
    pub installed_at: String,
    #[serde(default)]
    pub nu_version_at_install: Option<String>,
    #[serde(default)]
    pub activated: bool,
    #[serde(default)]
    pub registry_url: Option<String>,
    #[serde(default)]
    pub registry_revision: Option<String>,
    #[serde(default)]
    pub index_sha256: Option<String>,
    #[serde(default)]
    pub signing_key_fingerprint: Option<String>,
    // Source-built fields
    #[serde(default)]
    pub git_url: Option<String>,
    #[serde(default)]
    pub git_rev: Option<String>,
    #[serde(default)]
    pub cargo_name: Option<String>,
    #[serde(default)]
    pub cargo_lock_sha256: Option<String>,
    #[serde(default)]
    pub built_sha256: Option<String>,
}

impl Lockfile {
    pub fn load(root: &PathBuf) -> Result<Self> {
        let lock_path = root.join("lockfile");
        if !lock_path.exists() {
            return Ok(Self::empty());
        }
        let content = std::fs::read_to_string(&lock_path)
            .with_context(|| format!("Failed to read {}", lock_path.display()))?;
        let lockfile: Lockfile = serde_json::from_str(&content)?;
        Ok(lockfile)
    }

    pub fn save(&self, root: &PathBuf) -> Result<()> {
        let lock_path = root.join("lockfile");
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&lock_path, content)?;
        Ok(())
    }

    pub fn snapshot(&self, root: &PathBuf) -> Result<String> {
        let timestamp = format!(
            "{:016}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );
        let snapshot_dir = root.join(format!("snapshots/{timestamp}"));
        std::fs::create_dir_all(&snapshot_dir)?;
        let lock_path = snapshot_dir.join("lockfile.json");
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(lock_path, content)?;
        Ok(timestamp)
    }

    pub fn empty() -> Self {
        Self {
            version: 1,
            generated_at: String::new(),
            nu_version: String::new(),
            platform: String::new(),
            packages: HashMap::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_lockfile_roundtrip() {
        let lock = Lockfile::empty();
        let json = serde_json::to_string_pretty(&lock).unwrap();
        let parsed: Lockfile = serde_json::from_str(&json).unwrap();
        assert!(parsed.is_empty());
    }

    #[test]
    fn lockfile_roundtrip_with_entry() {
        let mut lock = Lockfile {
            version: 1,
            generated_at: "2026-06-27T12:00:00Z".to_string(),
            nu_version: "0.113.1".to_string(),
            platform: "x86_64-pc-windows-msvc".to_string(),
            packages: HashMap::new(),
        };

        lock.packages.insert(
            "test/pkg".to_string(),
            LockfileEntry {
                version: "1.0.0".to_string(),
                package_type: "plugin".to_string(),
                source: "binary".to_string(),
                target: Some("x86_64-pc-windows-msvc".to_string()),
                artifact_url: Some("https://example.com/pkg.zip".to_string()),
                artifact_sha256: Some("abc123".to_string()),
                executable_path: Some("nu_plugin_test.exe".to_string()),
                archive_root: None,
                include: None,
                entry: None,
                installed_at: "2026-06-27T12:00:00Z".to_string(),
                nu_version_at_install: Some("0.113.1".to_string()),
                activated: false,
                registry_url: Some("https://github.com/numan/numan-registry".to_string()),
                registry_revision: Some("abc123".to_string()),
                index_sha256: Some("def456".to_string()),
                signing_key_fingerprint: Some("sha256:789".to_string()),
                git_url: None,
                git_rev: None,
                cargo_name: None,
                cargo_lock_sha256: None,
                built_sha256: None,
            },
        );

        let json = serde_json::to_string_pretty(&lock).unwrap();
        let parsed: Lockfile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.packages.len(), 1);
        assert!(parsed.packages.contains_key("test/pkg"));
    }
}
