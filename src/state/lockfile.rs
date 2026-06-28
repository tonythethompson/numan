use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use crate::core::package::ModuleImportMode;
use crate::util::atomic::write_json_atomic;

/// Per-Nu-identity activation record stored on a plugin lockfile entry.
///
/// A plugin is "currently active" only when this record's hash, version, and
/// registry path all match the loaded `NuPaths`. A bare boolean would become
/// stale after `numan init --refresh` changes the Nu binary or registry target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginActivation {
    pub plugin_registry_path: String,
    pub nu_executable_sha256: String,
    pub nu_version: String,
    pub activated_at: String,
}

/// Per-Nu-identity activation record stored on a module lockfile entry.
///
/// A module is "currently active" only when this record's Nu executable hash,
/// Nu version, vendor-autoload directory, and managed file path all match the
/// cached `NuPaths` and the autoload-state projection. This is separate from
/// the plugin activation record because module activation is tied to a
/// vendor-autoload file rather than the Nu plugin registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleActivation {
    /// Absolute path to the entry `.nu` file within the installed payload.
    pub entry_path: String,
    /// Whether this module is imported namespaced or glob-imported.
    pub import_mode: ModuleImportMode,
    /// Absolute path to the selected vendor-autoload directory.
    pub vendor_autoload_dir: String,
    /// Absolute path to the Numan-managed autoload file (`numan.nu`).
    pub managed_file_path: String,
    /// SHA-256 of the Nu executable used when this activation was recorded.
    pub nu_executable_sha256: String,
    /// Version string of the Nu executable used when this activation was recorded.
    pub nu_version: String,
    /// Timestamp when this activation record was written.
    pub activated_at: String,
}

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
    /// Activation record for this plugin under a specific Nu identity.
    /// `None` means not yet activated (or activation record not yet written).
    /// Old JSON with `"activated": false` deserializes cleanly — serde ignores
    /// the unknown field; this field defaults to `None` via `#[serde(default)]`.
    #[serde(default)]
    pub activation: Option<PluginActivation>,
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
    /// Relative path to the installed payload directory from the numan root.
    /// E.g., "packages/plugins/fdncred/file/0.25.2-abc12345"
    #[serde(default)]
    pub payload_path: String,

    // Module-specific fields (Phase 4)
    /// Activation record for this module under a specific Nu identity and
    /// vendor-autoload target. `None` means not yet activated.
    /// This field is distinct from the plugin `activation` field — do not
    /// reinterpret or merge them.
    #[serde(default)]
    pub module_activation: Option<ModuleActivation>,

    /// Import mode persisted from the registry at install time.
    /// `None` for packages that have no `activation` spec (plugins, scripts,
    /// completions). `Some(ModuleImportMode::Module)` is the implicit default
    /// for module packages whose registry entry omits `activation.import`.
    #[serde(default)]
    pub module_import_mode: Option<ModuleImportMode>,

    /// Registry dependency map captured at install time.
    /// Activation requires this to be empty in Phase 4.
    #[serde(default)]
    pub locked_dependencies: BTreeMap<String, String>,
}

impl LockfileEntry {
    pub fn payload_path(&self) -> &str {
        &self.payload_path
    }

    /// Returns `true` if this plugin entry is active for the given Nu identity.
    pub fn is_active_for(
        &self,
        nu_executable_sha256: &str,
        nu_version: &str,
        plugin_registry_path: &str,
    ) -> bool {
        match &self.activation {
            Some(a) => {
                a.nu_executable_sha256 == nu_executable_sha256
                    && a.nu_version == nu_version
                    && a.plugin_registry_path == plugin_registry_path
            }
            None => false,
        }
    }

    /// Returns `true` if this module entry is active for the given Nu identity
    /// and vendor-autoload target. All four identity fields must match.
    pub fn is_module_active_for(
        &self,
        nu_executable_sha256: &str,
        nu_version: &str,
        vendor_autoload_dir: &str,
        managed_file_path: &str,
    ) -> bool {
        match &self.module_activation {
            Some(a) => {
                a.nu_executable_sha256 == nu_executable_sha256
                    && a.nu_version == nu_version
                    && a.vendor_autoload_dir == vendor_autoload_dir
                    && a.managed_file_path == managed_file_path
            }
            None => false,
        }
    }

    /// Returns `true` when this module can be activated in Phase 4:
    /// - `module_import_mode` is known (set at install time),
    /// - `entry` is set (required for `use` statement generation),
    /// - `locked_dependencies` is empty (Phase 4 cannot resolve cross-package deps).
    pub fn is_module_activatable(&self) -> bool {
        self.module_import_mode.is_some()
            && self.entry.is_some()
            && self.locked_dependencies.is_empty()
    }
}

impl Lockfile {
    pub fn load(root: &Path) -> Result<Self> {
        let lock_path = root.join("lockfile");
        if !lock_path.exists() {
            return Ok(Self::empty());
        }
        let content = std::fs::read_to_string(&lock_path)
            .with_context(|| format!("Failed to read {}", lock_path.display()))?;
        let lockfile: Lockfile = serde_json::from_str(&content)?;
        Ok(lockfile)
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let lock_path = root.join("lockfile");
        write_json_atomic(&lock_path, self)
    }

    pub fn snapshot(&self, root: &Path) -> Result<String> {
        let timestamp = crate::util::format_timestamp();
        let snapshot_dir = root.join(format!("snapshots/{timestamp}"));
        std::fs::create_dir_all(&snapshot_dir)?;
        let lock_path = snapshot_dir.join("lockfile.json");
        write_json_atomic(&lock_path, self)?;
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
                activation: None,
                registry_url: Some("registry:official".to_string()),
                registry_revision: Some("abc123".to_string()),
                index_sha256: Some("def456".to_string()),
                signing_key_fingerprint: Some("sha256:789".to_string()),
                git_url: None,
                git_rev: None,
                cargo_name: None,
                cargo_lock_sha256: None,
                built_sha256: None,
                payload_path: "packages/plugins/test/pkg/1.0.0-abc12345".to_string(),
                module_activation: None,
                module_import_mode: None,
                locked_dependencies: BTreeMap::new(),
            },
        );

        let json = serde_json::to_string_pretty(&lock).unwrap();
        let parsed: Lockfile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.packages.len(), 1);
        let entry = parsed.packages.get("test/pkg").unwrap();
        assert_eq!(
            entry.payload_path,
            "packages/plugins/test/pkg/1.0.0-abc12345"
        );
        assert!(entry.activation.is_none());
        assert!(entry.module_activation.is_none());
        assert!(entry.module_import_mode.is_none());
        assert!(entry.locked_dependencies.is_empty());
    }

    #[test]
    fn old_json_with_activated_bool_deserializes() {
        // JSON from before the PluginActivation migration — `"activated": false`
        // must deserialize without error; unknown fields are ignored by serde.
        let json = r#"{
            "version": 1,
            "generated_at": "",
            "nu_version": "0.113.1",
            "platform": "x86_64-pc-windows-msvc",
            "packages": {
                "test/pkg": {
                    "version": "1.0.0",
                    "type": "plugin",
                    "source": "binary",
                    "installed_at": "0000000000000000",
                    "activated": false,
                    "payload_path": "packages/plugins/test/pkg/1.0.0-abc"
                }
            }
        }"#;
        let parsed: Lockfile = serde_json::from_str(json).unwrap();
        let entry = parsed.packages.get("test/pkg").unwrap();
        assert!(entry.activation.is_none());
    }

    fn make_base_entry() -> LockfileEntry {
        LockfileEntry {
            version: "1.0.0".to_string(),
            package_type: "plugin".to_string(),
            source: "binary".to_string(),
            target: None,
            artifact_url: None,
            artifact_sha256: None,
            executable_path: None,
            archive_root: None,
            include: None,
            entry: None,
            installed_at: "0".to_string(),
            nu_version_at_install: None,
            activation: None,
            registry_url: None,
            registry_revision: None,
            index_sha256: None,
            signing_key_fingerprint: None,
            git_url: None,
            git_rev: None,
            cargo_name: None,
            cargo_lock_sha256: None,
            built_sha256: None,
            payload_path: String::new(),
            module_activation: None,
            module_import_mode: None,
            locked_dependencies: BTreeMap::new(),
        }
    }

    #[test]
    fn is_active_for_matches_correctly() {
        let entry = LockfileEntry {
            activation: Some(PluginActivation {
                plugin_registry_path: "/path/to/plugins.msgpackz".to_string(),
                nu_executable_sha256: "abc123".to_string(),
                nu_version: "0.113.1".to_string(),
                activated_at: "0".to_string(),
            }),
            ..make_base_entry()
        };

        assert!(entry.is_active_for("abc123", "0.113.1", "/path/to/plugins.msgpackz"));
        assert!(!entry.is_active_for("different_hash", "0.113.1", "/path/to/plugins.msgpackz"));
        assert!(!entry.is_active_for("abc123", "0.114.0", "/path/to/plugins.msgpackz"));
        assert!(!entry.is_active_for("abc123", "0.113.1", "/other/path.msgpackz"));
    }

    #[test]
    fn is_module_active_for_matches_correctly() {
        use crate::core::package::ModuleImportMode;

        let entry = LockfileEntry {
            package_type: "module".to_string(),
            module_activation: Some(ModuleActivation {
                entry_path: "/root/packages/modules/owner/foo/1.0.0-abc/mod.nu".to_string(),
                import_mode: ModuleImportMode::Module,
                vendor_autoload_dir: "/nu/vendor/autoload".to_string(),
                managed_file_path: "/nu/vendor/autoload/numan.nu".to_string(),
                nu_executable_sha256: "exe-hash".to_string(),
                nu_version: "0.113.1".to_string(),
                activated_at: "0".to_string(),
            }),
            module_import_mode: Some(ModuleImportMode::Module),
            ..make_base_entry()
        };

        assert!(entry.is_module_active_for(
            "exe-hash",
            "0.113.1",
            "/nu/vendor/autoload",
            "/nu/vendor/autoload/numan.nu"
        ));
        assert!(!entry.is_module_active_for(
            "wrong-hash",
            "0.113.1",
            "/nu/vendor/autoload",
            "/nu/vendor/autoload/numan.nu"
        ));
        assert!(!entry.is_module_active_for(
            "exe-hash",
            "0.114.0",
            "/nu/vendor/autoload",
            "/nu/vendor/autoload/numan.nu"
        ));
        assert!(!entry.is_module_active_for(
            "exe-hash",
            "0.113.1",
            "/other/vendor/autoload",
            "/nu/vendor/autoload/numan.nu"
        ));
        assert!(!entry.is_module_active_for(
            "exe-hash",
            "0.113.1",
            "/nu/vendor/autoload",
            "/other/path/numan.nu"
        ));
    }

    #[test]
    fn is_module_activatable_requires_mode_entry_and_no_deps() {
        use crate::core::package::ModuleImportMode;

        // All three conditions met
        let ready = LockfileEntry {
            package_type: "module".to_string(),
            entry: Some("mod.nu".to_string()),
            module_import_mode: Some(ModuleImportMode::Module),
            locked_dependencies: BTreeMap::new(),
            ..make_base_entry()
        };
        assert!(ready.is_module_activatable());

        // Missing entry
        let no_entry = LockfileEntry {
            package_type: "module".to_string(),
            entry: None,
            module_import_mode: Some(ModuleImportMode::Module),
            ..make_base_entry()
        };
        assert!(!no_entry.is_module_activatable());

        // Missing import mode
        let no_mode = LockfileEntry {
            package_type: "module".to_string(),
            entry: Some("mod.nu".to_string()),
            module_import_mode: None,
            ..make_base_entry()
        };
        assert!(!no_mode.is_module_activatable());

        // Has dependencies
        let with_deps = LockfileEntry {
            package_type: "module".to_string(),
            entry: Some("mod.nu".to_string()),
            module_import_mode: Some(ModuleImportMode::All),
            locked_dependencies: {
                let mut m = BTreeMap::new();
                m.insert("owner/dep".to_string(), "^1.0.0".to_string());
                m
            },
            ..make_base_entry()
        };
        assert!(!with_deps.is_module_activatable());
    }

    #[test]
    fn module_fields_survive_lockfile_roundtrip() {
        use crate::core::package::ModuleImportMode;

        let mut lock = Lockfile {
            version: 1,
            generated_at: "ts".to_string(),
            nu_version: "0.113.1".to_string(),
            platform: "x86_64-linux-gnu".to_string(),
            packages: HashMap::new(),
        };

        let mut deps = BTreeMap::new();
        deps.insert("owner/lib".to_string(), "^2.0.0".to_string());

        lock.packages.insert(
            "owner/mymod".to_string(),
            LockfileEntry {
                package_type: "module".to_string(),
                source: "archive".to_string(),
                entry: Some("mod.nu".to_string()),
                module_import_mode: Some(ModuleImportMode::All),
                locked_dependencies: deps,
                module_activation: Some(ModuleActivation {
                    entry_path: "/root/packages/modules/owner/mymod/1.0.0-aaa/mod.nu".to_string(),
                    import_mode: ModuleImportMode::All,
                    vendor_autoload_dir: "/nu/vendor/autoload".to_string(),
                    managed_file_path: "/nu/vendor/autoload/numan.nu".to_string(),
                    nu_executable_sha256: "abc".to_string(),
                    nu_version: "0.113.1".to_string(),
                    activated_at: "ts".to_string(),
                }),
                payload_path: "packages/modules/owner/mymod/1.0.0-aaa".to_string(),
                ..make_base_entry()
            },
        );

        let json = serde_json::to_string_pretty(&lock).unwrap();
        let parsed: Lockfile = serde_json::from_str(&json).unwrap();
        let entry = parsed.packages.get("owner/mymod").unwrap();

        assert_eq!(entry.module_import_mode, Some(ModuleImportMode::All));
        assert_eq!(entry.locked_dependencies.len(), 1);
        assert_eq!(
            entry.locked_dependencies.get("owner/lib").unwrap(),
            "^2.0.0"
        );

        let ma = entry.module_activation.as_ref().unwrap();
        assert_eq!(ma.import_mode, ModuleImportMode::All);
        assert_eq!(ma.vendor_autoload_dir, "/nu/vendor/autoload");
        assert_eq!(ma.managed_file_path, "/nu/vendor/autoload/numan.nu");
    }

    #[test]
    fn old_json_without_module_fields_deserializes_with_defaults() {
        // Existing lockfile entries written before Phase 4 must still parse.
        // All three new fields have #[serde(default)] and default to None/empty.
        let json = r#"{
            "version": 1,
            "generated_at": "",
            "nu_version": "0.113.1",
            "platform": "x86_64-pc-windows-msvc",
            "packages": {
                "test/pkg": {
                    "version": "1.0.0",
                    "type": "plugin",
                    "source": "binary",
                    "installed_at": "0000000000000000",
                    "payload_path": "packages/plugins/test/pkg/1.0.0-abc"
                }
            }
        }"#;
        let parsed: Lockfile = serde_json::from_str(json).unwrap();
        let entry = parsed.packages.get("test/pkg").unwrap();
        assert!(entry.module_activation.is_none());
        assert!(entry.module_import_mode.is_none());
        assert!(entry.locked_dependencies.is_empty());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let lock = Lockfile {
            version: 1,
            generated_at: "ts".to_string(),
            nu_version: "0.113.1".to_string(),
            platform: "x86_64-pc-windows-msvc".to_string(),
            packages: HashMap::new(),
        };
        lock.save(&root).unwrap();
        let loaded = Lockfile::load(&root).unwrap();
        assert_eq!(loaded.nu_version, "0.113.1");
    }
}
