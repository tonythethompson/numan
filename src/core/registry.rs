use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

use crate::core::integrity;
use crate::core::package::{Package, RegistryIndex};
use crate::core::trust::TrustStore;

pub struct RegistryManager {
    root: PathBuf,
    trust: TrustStore,
}

/// Registry index loaded with signature policy applied.
pub struct VerifiedRegistry {
    pub index: RegistryIndex,
    pub registry_name: String,
    pub index_sha256: String,
    pub signing_key_fingerprint: Option<String>,
}

impl RegistryManager {
    pub fn new(root: &Path) -> Result<Self> {
        let trust = TrustStore::load(root)?;
        Ok(Self {
            root: root.to_path_buf(),
            trust,
        })
    }

    pub fn index_path(&self, registry_name: &str) -> PathBuf {
        self.root
            .join(format!("registry/{registry_name}/index.json"))
    }

    pub fn sig_path(&self, registry_name: &str) -> PathBuf {
        self.root
            .join(format!("registry/{registry_name}/index.json.sig"))
    }

    pub fn load_index(&self, registry_name: &str) -> Result<RegistryIndex> {
        let path = self.index_path(registry_name);
        let content = std::fs::read_to_string(&path).with_context(|| {
            format!("Registry '{registry_name}' not synced. Run 'numan registry sync'.")
        })?;
        let index: RegistryIndex = serde_json::from_str(&content)?;
        Ok(index)
    }

    pub fn load_index_from_str(&self, content: &str) -> Result<RegistryIndex> {
        let index: RegistryIndex = serde_json::from_str(content)?;
        Ok(index)
    }

    pub fn save_index(&self, registry_name: &str, index: &RegistryIndex) -> Result<()> {
        let path = self.index_path(registry_name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(index)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    pub fn save_signature(&self, registry_name: &str, sig_b64: &str) -> Result<()> {
        let path = self.sig_path(registry_name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, sig_b64)?;
        Ok(())
    }

    pub fn verify_and_load(&self, registry_name: &str) -> Result<RegistryIndex> {
        let index = self.load_index(registry_name)?;
        let sig_path = self.sig_path(registry_name);

        if sig_path.exists() {
            let sig_b64 = std::fs::read_to_string(&sig_path)?;
            let index_content = std::fs::read(self.index_path(registry_name))?;

            if !self.trust.keys.contains_key(registry_name) {
                anyhow::bail!(
                    "No trusted key for registry '{registry_name}'. \
                     Run 'numan registry add' with --key first."
                );
            }

            let valid = self
                .trust
                .verify_signature(registry_name, &index_content, &sig_b64)?;
            if !valid {
                anyhow::bail!(
                    "Registry '{registry_name}' signature verification failed. \
                     The index may have been tampered with."
                );
            }
        }

        Ok(index)
    }

    pub fn search(&self, query: &str) -> Result<Vec<Package>> {
        let index = self.load_index(&self.default_registry())?;
        let query_lower = query.to_lowercase();

        let results: Vec<Package> = index
            .packages
            .into_iter()
            .filter(|p| {
                p.id.to_string().to_lowercase().contains(&query_lower)
                    || p.description.to_lowercase().contains(&query_lower)
                    || p.tags
                        .iter()
                        .any(|t| t.to_lowercase().contains(&query_lower))
            })
            .collect();

        Ok(results)
    }

    pub fn find_package(&self, id: &str) -> Result<Option<Package>> {
        let index = self.load_index(&self.default_registry())?;
        Ok(index.packages.into_iter().find(|p| p.id.to_string() == id))
    }

    fn default_registry(&self) -> String {
        let config_path = self.root.join("config.toml");
        if config_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&config_path) {
                if let Ok(config) = toml::from_str::<crate::config::Config>(&content) {
                    return config.general.default_registry;
                }
            }
        }
        "official".to_string()
    }

    /// Get the default registry name (public).
    pub fn default_registry_name(&self) -> String {
        self.default_registry()
    }

    /// Get the signing key fingerprint for a registry, if a trusted key exists.
    pub fn signing_key_fingerprint(&self, registry_name: &str) -> Option<String> {
        self.trust
            .keys
            .get(registry_name)
            .map(|k| k.fingerprint.clone())
    }

    /// Load a registry index, enforcing signature verification when a sig file exists.
    pub fn load_verified(&self, registry_name: &str) -> Result<VerifiedRegistry> {
        let sig_path = self.sig_path(registry_name);
        if sig_path.exists() {
            let index = self.verify_and_load(registry_name)?;
            let index_bytes = std::fs::read(self.index_path(registry_name))?;
            let index_sha256 = integrity::compute_sha256(&index_bytes);
            let fingerprint = self.signing_key_fingerprint(registry_name);
            Ok(VerifiedRegistry {
                index,
                registry_name: registry_name.to_string(),
                index_sha256,
                signing_key_fingerprint: fingerprint,
            })
        } else if std::env::var("NUMAN_ALLOW_UNSIGNED").unwrap_or_default() != "1" {
            bail!(
                "Registry '{}' has no signature file. \
                 Signatures are required by default. \
                 Set NUMAN_ALLOW_UNSIGNED=1 to override (development only).",
                registry_name
            );
        } else {
            let index = self.load_index(registry_name)?;
            let index_bytes = std::fs::read(self.index_path(registry_name))?;
            let index_sha256 = integrity::compute_sha256(&index_bytes);
            Ok(VerifiedRegistry {
                index,
                registry_name: registry_name.to_string(),
                index_sha256,
                signing_key_fingerprint: None,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::package::*;
    use std::collections::{BTreeMap, HashMap};

    fn test_index() -> RegistryIndex {
        RegistryIndex {
            version: 1,
            updated_at: "2026-06-27T00:00:00Z".to_string(),
            registry_revision: Some("abc123".to_string()),
            packages: vec![Package {
                id: ScopedId::new("test", "pkg"),
                description: "A test package".to_string(),
                repo: "https://github.com/test/pkg".to_string(),
                package_type: PackageType::Plugin,
                tags: vec!["test".to_string()],
                versions: vec![VersionEntry {
                    version: semver::Version::new(1, 0, 0),
                    nu_version: ">=0.113.0 <0.114.0".to_string(),
                    verified_with: vec![],
                    artifact: Artifact {
                        kind: "binary".to_string(),
                        url: None,
                        sha256: None,
                        targets: HashMap::new(),
                        archive_root: None,
                        include: None,
                        entry: None,
                    },
                    source: None,
                    dependencies: BTreeMap::new(),
                    activation: None,
                }],
            }],
        }
    }

    fn setup_root() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        std::fs::create_dir_all(root.join("registry/official")).unwrap();

        let index = test_index();
        let content = serde_json::to_string_pretty(&index).unwrap();
        std::fs::write(root.join("registry/official/index.json"), content).unwrap();

        std::fs::write(
            root.join("config.toml"),
            "[general]\ndefault_registry = \"official\"\n",
        )
        .unwrap();

        tmp
    }

    #[test]
    fn search_finds_by_id() {
        let tmp = setup_root();
        let root = tmp.path().to_path_buf();
        let mgr = RegistryManager::new(&root).unwrap();
        let results = mgr.search("pkg").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id.to_string(), "test/pkg");
    }

    #[test]
    fn search_finds_by_tag() {
        let tmp = setup_root();
        let root = tmp.path().to_path_buf();
        let mgr = RegistryManager::new(&root).unwrap();
        let results = mgr.search("test").unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn search_no_results() {
        let tmp = setup_root();
        let root = tmp.path().to_path_buf();
        let mgr = RegistryManager::new(&root).unwrap();
        let results = mgr.search("nonexistent").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn find_package_exact_match() {
        let tmp = setup_root();
        let root = tmp.path().to_path_buf();
        let mgr = RegistryManager::new(&root).unwrap();
        let pkg = mgr.find_package("test/pkg").unwrap();
        assert!(pkg.is_some());
        assert_eq!(pkg.unwrap().id.to_string(), "test/pkg");
    }

    #[test]
    fn find_package_not_found() {
        let tmp = setup_root();
        let root = tmp.path().to_path_buf();
        let mgr = RegistryManager::new(&root).unwrap();
        let pkg = mgr.find_package("test/nonexistent").unwrap();
        assert!(pkg.is_none());
    }
}
