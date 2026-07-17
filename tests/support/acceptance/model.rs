use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const EVIDENCE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stage1Config {
    pub schema_version: u32,
    pub output_base: PathBuf,
    pub package_id: String,
    pub query: String,
}

impl Stage1Config {
    pub fn from_env() -> Result<Self> {
        let output_base = std::env::var_os("NUMAN_ACCEPTANCE_OUTPUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("target/acceptance/official-registry-stage1"));
        let package_id = std::env::var("NUMAN_ACCEPTANCE_PACKAGE")
            .unwrap_or_else(|_| "fdncred/nu_plugin_file".to_string());
        let query = std::env::var("NUMAN_ACCEPTANCE_QUERY")
            .unwrap_or_else(|_| "nu_plugin_file".to_string());
        anyhow::ensure!(
            package_id.split('/').count() == 2,
            "NUMAN_ACCEPTANCE_PACKAGE must be an exact owner/name package ID"
        );
        anyhow::ensure!(
            !query.trim().is_empty(),
            "acceptance query must not be empty"
        );
        Ok(Self {
            schema_version: EVIDENCE_SCHEMA_VERSION,
            output_base,
            package_id,
            query,
        })
    }

    pub fn timeout_for(step: &str) -> Duration {
        match step {
            "registry-sync" | "activate" => Duration::from_secs(120),
            "install" => Duration::from_secs(300),
            _ => Duration::from_secs(60),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildEnvironment {
    pub schema_version: u32,
    pub os: String,
    pub architecture: String,
    pub path_entry_count: usize,
    pub isolated_paths: BTreeMap<String, String>,
    pub removed_variables: Vec<String>,
    #[serde(skip)]
    pub variables: BTreeMap<String, String>,
}

impl ChildEnvironment {
    pub fn isolated(home: &Path) -> Result<Self> {
        let home = absolute_path(home)?;
        let paths = [
            ("HOME", home.clone()),
            ("USERPROFILE", home.clone()),
            ("APPDATA", home.join("appdata/roaming")),
            ("LOCALAPPDATA", home.join("appdata/local")),
            ("XDG_CONFIG_HOME", home.join("xdg/config")),
            ("XDG_DATA_HOME", home.join("xdg/data")),
            ("XDG_CACHE_HOME", home.join("xdg/cache")),
            ("TEMP", home.join("temp")),
            ("TMP", home.join("temp")),
        ];
        for (_, path) in &paths {
            std::fs::create_dir_all(path)
                .with_context(|| format!("failed to create isolated path {}", path.display()))?;
        }
        for path in [
            home.join("appdata/roaming/nushell"),
            home.join("appdata/local/nushell"),
            home.join("xdg/config/nushell"),
            home.join("xdg/data/nushell"),
            home.join("xdg/cache/nushell"),
        ] {
            std::fs::create_dir_all(&path).with_context(|| {
                format!(
                    "failed to create isolated Nushell parent {}",
                    path.display()
                )
            })?;
        }

        let parent: BTreeMap<String, String> = std::env::vars().collect();
        let mut variables = BTreeMap::new();
        for name in [
            "PATH",
            "SystemRoot",
            "WINDIR",
            "SystemDrive",
            "ComSpec",
            "PATHEXT",
        ] {
            if let Some((actual, value)) = parent
                .iter()
                .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            {
                variables.insert(actual.clone(), value.clone());
            }
        }
        let path_value = variables
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("PATH"))
            .map(|(_, value)| value.as_str())
            .unwrap_or_default();
        let path_entry_count = std::env::split_paths(path_value).count();

        let mut isolated_paths = BTreeMap::new();
        for (name, path) in paths {
            let value = path.to_string_lossy().into_owned();
            variables.insert(name.to_string(), value.clone());
            isolated_paths.insert(name.to_string(), value);
        }
        variables.insert("NO_COLOR".to_string(), "1".to_string());

        let mut removed_variables: Vec<String> = parent
            .keys()
            .filter(|name| !variables.keys().any(|kept| kept.eq_ignore_ascii_case(name)))
            .cloned()
            .collect();
        removed_variables.sort_by_key(|name| name.to_ascii_uppercase());

        Ok(Self {
            schema_version: EVIDENCE_SCHEMA_VERSION,
            os: std::env::consts::OS.to_string(),
            architecture: std::env::consts::ARCH.to_string(),
            path_entry_count,
            isolated_paths,
            removed_variables,
            variables,
        })
    }

    pub fn new_for_test(additional: BTreeMap<String, String>) -> Self {
        let mut variables = BTreeMap::new();
        for name in [
            "PATH",
            "SystemRoot",
            "WINDIR",
            "SystemDrive",
            "ComSpec",
            "PATHEXT",
        ] {
            if let Some((actual, value)) =
                std::env::vars().find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            {
                variables.insert(actual, value);
            }
        }
        variables.extend(additional);
        let path_entry_count = variables
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("PATH"))
            .map(|(_, value)| std::env::split_paths(value).count())
            .unwrap_or_default();
        Self {
            schema_version: EVIDENCE_SCHEMA_VERSION,
            os: std::env::consts::OS.to_string(),
            architecture: std::env::consts::ARCH.to_string(),
            path_entry_count,
            isolated_paths: BTreeMap::new(),
            removed_variables: Vec::new(),
            variables,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandSpec {
    pub schema_version: u32,
    pub step: String,
    pub program: PathBuf,
    pub arguments: Vec<String>,
    pub timeout_ms: u64,
}

impl CommandSpec {
    pub fn new(
        step: impl Into<String>,
        program: PathBuf,
        arguments: Vec<String>,
        timeout: Duration,
    ) -> Self {
        Self {
            schema_version: EVIDENCE_SCHEMA_VERSION,
            step: step.into(),
            program,
            arguments,
            timeout_ms: timeout.as_millis().try_into().unwrap_or(u64::MAX),
        }
    }

    pub fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandOutcome {
    pub schema_version: u32,
    pub started_utc_ms: u128,
    pub finished_utc_ms: u128,
    pub duration_ms: u128,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub stdout_sha256: String,
    pub stderr_sha256: String,
    #[serde(skip)]
    pub stdout: Vec<u8>,
    #[serde(skip)]
    pub stderr: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InventoryKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InventoryEntry {
    pub path: String,
    pub kind: InventoryKind,
    pub size: u64,
    pub sha256: Option<String>,
    pub symlink_target: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateFileEvidence {
    pub path: String,
    pub exists: bool,
    pub size: Option<u64>,
    pub sha256: Option<String>,
    pub parsed: Option<serde_json::Value>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateEvidence {
    pub schema_version: u32,
    pub captured_utc_ms: u128,
    pub files: Vec<StateFileEvidence>,
    pub journals: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PayloadReference {
    pub source: String,
    pub package_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackageDirectoryEvidence {
    pub path: String,
    pub references: Vec<PayloadReference>,
    pub orphan: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepSummary {
    pub step: String,
    pub passed: bool,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub assertion_errors: Vec<String>,
    pub evidence_directory: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSummary {
    pub schema_version: u32,
    pub run_id: String,
    pub status: String,
    pub package_id: String,
    pub query: String,
    pub resolved_version: Option<String>,
    pub doctor_errors: Option<u64>,
    pub doctor_warnings: Option<u64>,
    pub steps: Vec<StepSummary>,
    pub remaining_payloads: Vec<PackageDirectoryEvidence>,
    pub evidence_directory: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub schema_version: u32,
    pub run_id: String,
    pub started_utc_ms: u128,
    pub finished_utc_ms: Option<u128>,
    pub status: String,
    pub config: Stage1Config,
    pub numan_binary: String,
    pub root: String,
    pub home: String,
    pub evidence: String,
    pub environment: ChildEnvironment,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptanceFailure {
    pub failed_step: String,
    pub arguments: Vec<String>,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub assertion_errors: Vec<String>,
    pub stdout_path: String,
    pub stderr_path: String,
    pub evidence_directory: String,
}

impl std::fmt::Display for AcceptanceFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "stage '{}' failed; args={:?}; exit={:?}; timeout={}; assertions={:?}; stdout={}; stderr={}; evidence={}",
            self.failed_step,
            self.arguments,
            self.exit_code,
            self.timed_out,
            self.assertion_errors,
            self.stdout_path,
            self.stderr_path,
            self.evidence_directory
        )
    }
}

impl std::error::Error for AcceptanceFailure {}

pub fn utc_unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock must be after Unix epoch")
        .as_millis()
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}
