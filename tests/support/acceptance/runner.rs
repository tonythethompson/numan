use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;
use uuid::Uuid;

use numan_cli::config::Config;
use numan_cli::core::nu_version::NuVersion;
use numan_cli::core::official_registry::{RegistrySignature, OFFICIAL_REGISTRY};
use numan_cli::core::package::PackageType;
use numan_cli::core::platform::Platform;
use numan_cli::core::registry::RegistryManager;
use numan_cli::core::resolve::Resolver;
use numan_cli::nu::paths::NuPaths;
use numan_cli::state::lockfile::{compute_revision_id, Lockfile};

use super::filesystem::{
    capture_state, classify_package_dirs, discover_journals, inventory_root, path_is_contained,
    sha256_file,
};
use super::model::{
    utc_unix_ms, AcceptanceFailure, ChildEnvironment, CommandOutcome, CommandSpec, RunRecord,
    RunSummary, Stage1Config, StepName, StepSummary, EVIDENCE_SCHEMA_VERSION,
};
use super::process::run_command;

pub struct AcceptanceRun {
    pub config: Stage1Config,
    pub numan_binary: PathBuf,
    pub run_id: String,
    pub run_dir: PathBuf,
    pub root: PathBuf,
    pub home: PathBuf,
    pub evidence: PathBuf,
    pub environment: ChildEnvironment,
    pub record: RunRecord,
    steps: Vec<StepSummary>,
    resolved_version: Option<String>,
    registry_key_id: Option<String>,
    registry_index_sha256: Option<String>,
    signing_key_fingerprint: Option<String>,
    executable_sha256: Option<String>,
    doctor_errors: Option<u64>,
    doctor_warnings: Option<u64>,
}

struct StepArtifacts {
    name: String,
    spec: CommandSpec,
    outcome: CommandOutcome,
    directory: PathBuf,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
    process_error: Option<String>,
}

#[derive(Serialize)]
struct CommandEvidence<'a> {
    schema_version: u32,
    command: &'a CommandSpec,
    outcome: &'a CommandOutcome,
    process_error: &'a Option<String>,
}

impl AcceptanceRun {
    pub fn new(config: Stage1Config, numan_binary: PathBuf) -> Result<Self> {
        let now = utc_unix_ms();
        let uuid = Uuid::new_v4().simple().to_string();
        let run_id = format!("{now}-{}", &uuid[..8]);
        let base = absolute_path(&config.output_base)?;
        let run_dir = base.join(&run_id);
        let root = run_dir.join("root");
        let home = run_dir.join("home");
        let evidence = run_dir.join("evidence");
        std::fs::create_dir_all(&home)?;
        std::fs::create_dir_all(&evidence)?;
        let environment = ChildEnvironment::isolated(&home)?;
        let record = RunRecord {
            schema_version: EVIDENCE_SCHEMA_VERSION,
            run_id: run_id.clone(),
            started_utc_ms: now,
            finished_utc_ms: None,
            status: "running".to_string(),
            config: config.clone(),
            numan_binary: numan_binary.to_string_lossy().into_owned(),
            root: root.to_string_lossy().into_owned(),
            home: home.to_string_lossy().into_owned(),
            evidence: evidence.to_string_lossy().into_owned(),
            environment: environment.clone(),
        };
        write_json(&evidence.join("run.json"), &record)?;
        Ok(Self {
            config,
            numan_binary,
            run_id,
            run_dir,
            root,
            home,
            evidence,
            environment,
            record,
            steps: Vec::new(),
            resolved_version: None,
            registry_key_id: None,
            registry_index_sha256: None,
            signing_key_fingerprint: None,
            executable_sha256: None,
            doctor_errors: None,
            doctor_warnings: None,
        })
    }

    pub fn execute(&mut self) -> Result<RunSummary, AcceptanceFailure> {
        if self.root.exists() {
            return Err(self.preflight_failure(vec![format!(
                "acceptance root existed before the first Numan invocation: {}",
                self.root.display()
            )]));
        }

        let preflight = self.required_external_step(
            0,
            StepName::Preflight,
            PathBuf::from("nu"),
            vec!["--version".to_string()],
            StepName::Preflight.timeout(),
        )?;
        let mut errors = basic_errors(&preflight);
        if std::env::consts::OS != "windows" || std::env::consts::ARCH != "x86_64" {
            errors.push(format!(
                "Stage 1 requires Windows x86_64, found {} {}",
                std::env::consts::OS,
                std::env::consts::ARCH
            ));
        }
        let detected_nu = String::from_utf8_lossy(&preflight.outcome.stdout)
            .trim()
            .to_string();
        let nu_version = match NuVersion::parse(&detected_nu) {
            Ok(version) if version.major == 0 && version.minor == 113 => Some(version),
            Ok(version) => {
                errors.push(format!(
                    "Stage 1 requires Nu 0.113.x, found {}",
                    version.version
                ));
                None
            }
            Err(error) => {
                errors.push(format!(
                    "failed to parse Nu version '{detected_nu}': {error}"
                ));
                None
            }
        };
        self.record_step(preflight, errors)?;
        let nu_version = nu_version.expect("successful preflight records a parsed Nu version");

        let init = self.required_numan_step(1, StepName::Init)?;
        let mut errors = basic_errors(&init);
        for directory in ["nu_state", "state", "packages", "registries"] {
            if !self.root.join(directory).is_dir() {
                errors.push(format!(
                    "init did not create expected directory '{directory}'"
                ));
            }
        }
        match Config::load(&self.root) {
            Ok(config) => {
                if config.registries.len() != 1 {
                    errors.push(format!(
                        "expected exactly one registry, found {}",
                        config.registries.len()
                    ));
                }
                match config.registries.get("official") {
                    Some(registry) => {
                        if !registry.enabled {
                            errors.push("official registry is not enabled".to_string());
                        }
                        if registry.url != OFFICIAL_REGISTRY.production_url {
                            errors
                                .push(format!("official registry URL mismatch: {}", registry.url));
                        }
                        if registry.trust_key.is_some() {
                            errors.push(
                                "official registry unexpectedly has an inline trust key"
                                    .to_string(),
                            );
                        }
                    }
                    None => errors.push("official registry is missing from config".to_string()),
                }
            }
            Err(error) => errors.push(format!("failed to parse config: {error}")),
        }
        let nu_paths = match NuPaths::load(&self.root) {
            Ok(paths) => {
                if paths.nu_version != nu_version.version {
                    errors.push(format!(
                        "cached Nu version {} does not match preflight {}",
                        paths.nu_version, nu_version.version
                    ));
                }
                self.validate_mutable_nu_paths(&paths, &mut errors);
                Some(paths)
            }
            Err(error) => {
                errors.push(format!("failed to parse nu_state/paths.json: {error}"));
                None
            }
        };
        self.record_step(init, errors)?;
        let nu_paths = nu_paths.expect("successful init records Nu paths");

        let sync = self.required_numan_step(2, StepName::RegistrySync)?;
        let mut errors = basic_errors(&sync);
        let stderr = String::from_utf8_lossy(&sync.outcome.stderr).to_ascii_lowercase();
        if stderr.contains("using cached") || stderr.contains("last-known-good") {
            errors.push(
                "registry sync used a fallback instead of a fresh official index".to_string(),
            );
        }
        let index_path = self.root.join("registry/official/index.json");
        let signature_path = self.root.join("registry/official/index.json.sig");
        if std::fs::metadata(&index_path)
            .map(|meta| meta.len())
            .unwrap_or(0)
            == 0
        {
            errors.push("official registry index is missing or empty".to_string());
        }
        if std::fs::metadata(&signature_path)
            .map(|meta| meta.len())
            .unwrap_or(0)
            == 0
        {
            errors.push("official registry signature is missing or empty".to_string());
        }
        let signature = read_json::<RegistrySignature>(&signature_path, &mut errors);
        if let Some(signature) = &signature {
            if signature.key_id == "unsigned" || signature.key_id.is_empty() {
                errors
                    .push("registry signature did not identify a verified signing key".to_string());
            }
        }

        let mut expected_artifact_sha256 = None;
        let mut expected_executable = None;
        let mut expected_package_type = None;
        match RegistryManager::new(&self.root).and_then(|manager| manager.load_verified("official"))
        {
            Ok(verified) => {
                self.registry_key_id = Some(verified.key_id.clone());
                self.registry_index_sha256 = Some(verified.index_sha256.clone());
                self.signing_key_fingerprint = verified.signing_key_fingerprint.clone();
                if verified.key_id == "unsigned" {
                    errors.push("verified registry reported unsigned provenance".to_string());
                }
                match &verified.signing_key_fingerprint {
                    Some(fingerprint)
                        if fingerprint.starts_with("sha256:") && fingerprint.len() == 71 => {}
                    Some(fingerprint) => errors.push(format!(
                        "invalid signing-key fingerprint representation: {fingerprint}"
                    )),
                    None => errors
                        .push("verified official signing-key fingerprint is absent".to_string()),
                }
                let package = verified
                    .index
                    .packages
                    .iter()
                    .find(|package| package.id.to_string() == self.config.package_id);
                match package {
                    Some(package) => {
                        if package.package_type != PackageType::Plugin {
                            errors.push(format!(
                                "override package must be an activatable plugin, found {}",
                                package.package_type
                            ));
                        }
                        let platform = Platform::detect();
                        match Resolver::new(&platform, &nu_version).resolve(package) {
                            Ok(version) => {
                                self.resolved_version = Some(version.version.to_string());
                                expected_package_type = Some(package.package_type.to_string());
                                match version.artifact.targets.get(&platform.triple) {
                                    Some(target) => {
                                        expected_artifact_sha256 = Some(target.sha256.clone());
                                        expected_executable = Some(target.executable_path.clone());
                                    }
                                    None => errors.push(format!(
                                        "resolved package lacks target {}",
                                        platform.triple
                                    )),
                                }
                            }
                            Err(error) => errors.push(format!(
                                "production resolver found no compatible version: {error}"
                            )),
                        }
                    }
                    None => errors.push(format!(
                        "exact package '{}' is absent from verified index",
                        self.config.package_id
                    )),
                }
            }
            Err(error) => errors.push(format!("failed to load verified registry: {error}")),
        }
        if signature.as_ref().map(|sig| sig.key_id.as_str()) != self.registry_key_id.as_deref() {
            errors.push("signature envelope key ID differs from verified key ID".to_string());
        }
        self.record_step(sync, errors)?;
        let resolved_version = self
            .resolved_version
            .clone()
            .expect("successful sync records resolved version");
        let expected_artifact_sha256 =
            expected_artifact_sha256.expect("successful sync records artifact hash");
        let expected_executable = expected_executable.expect("successful sync records executable");
        let expected_package_type =
            expected_package_type.expect("successful sync records package type");

        let search = self.required_numan_step(3, StepName::Search)?;
        let mut errors = basic_errors(&search);
        let stdout = String::from_utf8_lossy(&search.outcome.stdout);
        if !stdout
            .lines()
            .any(|line| line.split_whitespace().next() == Some(self.config.package_id.as_str()))
        {
            errors.push(format!(
                "search output has no exact package-ID row for {}",
                self.config.package_id
            ));
        }
        match RegistryManager::new(&self.root)
            .and_then(|manager| manager.find_package(&self.config.package_id))
        {
            Ok(Some(_)) => {}
            Ok(None) => errors.push("cached index lost the exact package after search".to_string()),
            Err(error) => errors.push(format!("failed to inspect cached index: {error}")),
        }
        self.record_step(search, errors)?;

        let info = self.required_numan_step(4, StepName::Info)?;

        let mut errors = basic_errors(&info);
        let stdout = String::from_utf8_lossy(&info.outcome.stdout);
        for expected in [
            format!("Package:    {}", self.config.package_id),
            format!("Type:       {expected_package_type}"),
            format!("v{resolved_version}"),
        ] {
            if !stdout.contains(&expected) {
                errors.push(format!("info output is missing exact text '{expected}'"));
            }
        }
        self.record_step(info, errors)?;

        let install = self.required_numan_step(5, StepName::Install)?;
        let mut errors = basic_errors(&install);
        match Lockfile::load(&self.root) {
            Ok(lockfile) => {
                if lockfile.packages.len() != 1 {
                    errors.push(format!(
                        "expected one lockfile package after install, found {}",
                        lockfile.packages.len()
                    ));
                }
                match lockfile.packages.get(&self.config.package_id) {
                    Some(entry) => {
                        if entry.version != resolved_version {
                            errors.push(format!(
                                "installed version {} differs from resolved {}",
                                entry.version, resolved_version
                            ));
                        }
                        if entry.package_type != expected_package_type {
                            errors.push(format!(
                                "installed type {} differs from expected {}",
                                entry.package_type, expected_package_type
                            ));
                        }
                        if entry.activation.is_some() || entry.module_activation.is_some() {
                            errors.push("fresh install is not activation-inert".to_string());
                        }
                        let payload = self.root.join(&entry.payload_path);
                        validate_contained_existing(&payload, &self.root, "payload", &mut errors);
                        if entry.registry_url.as_deref() != Some("registry:official")
                            || entry.origin.as_deref() != Some("registry:official")
                        {
                            errors.push(
                                "official registry/origin provenance is incomplete".to_string(),
                            );
                        }
                        if entry
                            .registry_revision
                            .as_deref()
                            .unwrap_or_default()
                            .is_empty()
                            || entry.index_sha256.as_deref().unwrap_or_default().is_empty()
                            || entry
                                .signing_key_fingerprint
                                .as_deref()
                                .unwrap_or_default()
                                .is_empty()
                        {
                            errors.push(
                                "registry revision/index/fingerprint provenance is incomplete"
                                    .to_string(),
                            );
                        }
                        if entry.index_sha256.as_deref() != self.registry_index_sha256.as_deref()
                            || entry.signing_key_fingerprint.as_deref()
                                != self.signing_key_fingerprint.as_deref()
                        {
                            errors.push(
                                "lockfile registry provenance differs from verified index"
                                    .to_string(),
                            );
                        }
                        if entry.artifact_sha256.as_deref() != Some(&expected_artifact_sha256) {
                            errors.push(
                                "lockfile artifact hash differs from registry target".to_string(),
                            );
                        }
                        let cache = self
                            .root
                            .join("cache/downloads")
                            .join(format!("{expected_artifact_sha256}.bin"));
                        match sha256_file(&cache) {
                            Ok(observed) => {
                                if observed != expected_artifact_sha256
                                    || entry.payload_sha256.as_deref() != Some(observed.as_str())
                                {
                                    errors.push(
                                        "artifact/payload archive hash validation failed"
                                            .to_string(),
                                    );
                                }
                            }
                            Err(error) => errors.push(format!(
                                "failed to hash downloaded payload archive: {error}"
                            )),
                        }
                        match compute_revision_id(&payload) {
                            Some(revision) if entry.revision_id.as_deref() == Some(&revision) => {}
                            Some(_) => {
                                errors.push("payload revision hash validation failed".to_string())
                            }
                            None => {
                                errors.push("could not compute payload revision hash".to_string())
                            }
                        }
                        if entry.executable_path.as_deref() != Some(&expected_executable) {
                            errors.push(
                                "lockfile executable path differs from registry target".to_string(),
                            );
                        }
                        let executable = payload.join(&expected_executable);
                        validate_contained_existing(
                            &executable,
                            &self.root,
                            "plugin executable",
                            &mut errors,
                        );
                        match sha256_file(&executable) {
                            Ok(hash) => self.executable_sha256 = Some(hash),
                            Err(error) => errors
                                .push(format!("failed to capture plugin executable hash: {error}")),
                        }
                    }
                    None => errors.push(format!(
                        "lockfile has no exact entry for {}",
                        self.config.package_id
                    )),
                }
            }
            Err(error) => errors.push(format!("failed to parse installed lockfile: {error}")),
        }
        self.record_step(install, errors)?;

        let plugin_registry = PathBuf::from(&nu_paths.plugin_registry_path);
        let plugin_registry_before = sha256_file(&plugin_registry).ok();
        let activate = self.required_numan_step(6, StepName::Activate)?;
        let mut errors = basic_errors(&activate);
        match Lockfile::load(&self.root) {
            Ok(lockfile) => match lockfile.packages.get(&self.config.package_id) {
                Some(entry) => match &entry.activation {
                    Some(activation) => {
                        if activation.nu_executable_sha256 != nu_paths.nu_executable_hash
                            || activation.nu_version != nu_paths.nu_version
                            || activation.plugin_registry_path != nu_paths.plugin_registry_path
                        {
                            errors.push(
                                "activation record does not match cached Nu identity".to_string(),
                            );
                        }
                    }
                    None => errors.push("plugin has no activation record".to_string()),
                },
                None => errors.push("activated package disappeared from lockfile".to_string()),
            },
            Err(error) => errors.push(format!("failed to parse activated lockfile: {error}")),
        }
        let plugin_registry_after = sha256_file(&plugin_registry).ok();
        if plugin_registry_after.is_none() || plugin_registry_after == plugin_registry_before {
            errors.push("isolated Nu plugin registry did not change during activation".to_string());
        }
        require_clear_journals(&self.root, &mut errors);
        self.record_step(activate, errors)?;

        let doctor = self.required_numan_step(7, StepName::Doctor)?;
        let mut errors = basic_errors(&doctor);
        match serde_json::from_slice::<serde_json::Value>(&doctor.outcome.stdout) {
            Ok(report) => {
                if report
                    .get("schema_version")
                    .and_then(serde_json::Value::as_u64)
                    != Some(1)
                {
                    errors.push("doctor report schema_version is not 1".to_string());
                }
                self.doctor_errors = report
                    .pointer("/summary/errors")
                    .and_then(serde_json::Value::as_u64);
                self.doctor_warnings = report
                    .pointer("/summary/warnings")
                    .and_then(serde_json::Value::as_u64);
                if self.doctor_errors != Some(0) || self.doctor_warnings != Some(0) {
                    errors.push(format!(
                        "doctor reported {:?} errors and {:?} warnings; warning allowlist is empty",
                        self.doctor_errors, self.doctor_warnings
                    ));
                }
                match report.get("root").and_then(serde_json::Value::as_str) {
                    Some(root) => {
                        let reported = PathBuf::from(root);
                        if !same_path(&reported, &self.root) {
                            errors.push(format!(
                                "doctor normalized root '{}' differs from acceptance root '{}'",
                                reported.display(),
                                self.root.display()
                            ));
                        }
                    }
                    None => errors.push("doctor report has no root".to_string()),
                }
            }
            Err(error) => errors.push(format!("failed to parse doctor JSON: {error}")),
        }
        self.record_step(doctor, errors)?;

        let list = self.required_numan_step(8, StepName::List)?;
        let mut errors = basic_errors(&list);
        let expected_row = format!(
            "{}  v{}  [{}]  activated",
            self.config.package_id, resolved_version, expected_package_type
        );
        if !String::from_utf8_lossy(&list.outcome.stdout)
            .lines()
            .any(|line| line.trim() == expected_row)
        {
            errors.push(format!("list output has no exact row '{expected_row}'"));
        }
        // Active-plugin remove is gated while activation remains. Deactivate
        // clears the record; then remove (no --force) and gc complete the lifecycle.
        match Lockfile::load(&self.root) {
            Ok(lockfile) => match lockfile.packages.get(&self.config.package_id) {
                Some(entry) if entry.activation.is_some() => {}
                Some(_) => errors.push(
                    "activated package lost its plugin activation record after list".to_string(),
                ),
                None => errors
                    .push("activated package disappeared from lockfile after list".to_string()),
            },
            Err(error) => errors.push(format!("failed to parse lockfile after list: {error}")),
        }
        self.record_step(list, errors)?;

        let deactivate = self.required_numan_step(9, StepName::Deactivate)?;
        let mut errors = basic_errors(&deactivate);
        match Lockfile::load(&self.root) {
            Ok(lockfile) => match lockfile.packages.get(&self.config.package_id) {
                Some(entry) if entry.activation.is_none() => {}
                Some(_) => errors
                    .push("plugin activation record still present after deactivate".to_string()),
                None => errors.push("package missing from lockfile after deactivate".to_string()),
            },
            Err(error) => errors.push(format!(
                "failed to parse lockfile after deactivate: {error}"
            )),
        }
        require_clear_journals(&self.root, &mut errors);
        self.record_step(deactivate, errors)?;

        let remove = self.required_numan_step(10, StepName::Remove)?;
        let mut errors = basic_errors(&remove);
        match Lockfile::load(&self.root) {
            Ok(lockfile) if !lockfile.packages.contains_key(&self.config.package_id) => {}
            Ok(_) => errors.push("removed package remains in lockfile".to_string()),
            Err(error) => errors.push(format!("lockfile is invalid after remove: {error}")),
        }
        require_clear_journals(&self.root, &mut errors);
        self.record_step(remove, errors)?;

        let gc = self.required_numan_step(11, StepName::Gc)?;
        let mut errors = basic_errors(&gc);
        match Lockfile::load(&self.root) {
            Ok(lockfile) if lockfile.packages.is_empty() => {}
            Ok(_) => errors.push("current lockfile is not empty after GC".to_string()),
            Err(error) => errors.push(format!("lockfile is invalid after GC: {error}")),
        }
        require_clear_journals(&self.root, &mut errors);
        match classify_package_dirs(&self.root) {
            Ok(classified) => {
                for package in &classified {
                    if package.orphan {
                        errors.push(format!(
                            "remaining package directory has no current/snapshot/journal reference: {}",
                            package.path
                        ));
                    }
                }
            }
            Err(error) => errors.push(format!("failed to classify package directories: {error}")),
        }
        self.record_step(gc, errors)?;

        let remaining_payloads = classify_package_dirs(&self.root).unwrap_or_default();
        let summary = self.summary("passed", remaining_payloads);
        if let Err(error) = self.finalize(&summary) {
            return Err(
                self.preflight_failure(vec![format!("failed to finalize successful run: {error}")])
            );
        }
        Ok(summary)
    }

    pub fn finalize(&mut self, summary: &RunSummary) -> Result<()> {
        self.record.status = summary.status.clone();
        self.record.finished_utc_ms = Some(utc_unix_ms());
        write_json(&self.evidence.join("run.json"), &self.record)?;
        write_json(&self.evidence.join("summary.json"), summary)?;
        std::fs::write(self.evidence.join("summary.md"), render_summary(summary))?;
        Ok(())
    }

    fn required_numan_step(
        &mut self,
        index: usize,
        step: StepName,
    ) -> Result<StepArtifacts, AcceptanceFailure> {
        let mut arguments = vec![
            "--root".to_string(),
            self.root.to_string_lossy().into_owned(),
        ];
        arguments.extend(step.command_args(&self.config));
        self.required_external_step(
            index,
            step,
            self.numan_binary.clone(),
            arguments,
            step.timeout(),
        )
    }

    fn required_external_step(
        &mut self,
        index: usize,
        step: StepName,
        program: PathBuf,
        arguments: Vec<String>,
        timeout: std::time::Duration,
    ) -> Result<StepArtifacts, AcceptanceFailure> {
        let name = step.as_str();
        match self.run_step(index, step, program, arguments.clone(), timeout) {
            Ok(step) => Ok(step),
            Err(error) => Err(self.preflight_failure(vec![format!(
                "could not persist evidence for step '{name}' with args {arguments:?}: {error}"
            )])),
        }
    }

    fn run_step(
        &self,
        index: usize,
        step: StepName,
        program: PathBuf,
        arguments: Vec<String>,
        timeout: std::time::Duration,
    ) -> Result<StepArtifacts> {
        let name = step.as_str();
        let directory = self.evidence.join(format!("{index:02}-{name}"));
        std::fs::create_dir_all(&directory)?;
        let spec = CommandSpec::new(step, program, arguments, timeout);
        let (outcome, process_error) = match run_command(&spec, &self.environment) {
            Ok(outcome) => (outcome, None),
            Err(error) => {
                let message = error.to_string();
                let hash = numan_cli::core::integrity::compute_sha256(message.as_bytes());
                (
                    CommandOutcome {
                        schema_version: EVIDENCE_SCHEMA_VERSION,
                        started_utc_ms: utc_unix_ms(),
                        finished_utc_ms: utc_unix_ms(),
                        duration_ms: 0,
                        exit_code: None,
                        timed_out: false,
                        stdout_bytes: 0,
                        stderr_bytes: message.len().try_into().unwrap_or(u64::MAX),
                        stdout_sha256: numan_cli::core::integrity::compute_sha256(&[]),
                        stderr_sha256: hash,
                        stdout: Vec::new(),
                        stderr: message.into_bytes(),
                    },
                    Some(error.to_string()),
                )
            }
        };
        let stdout_path = directory.join("stdout.txt");
        let stderr_path = directory.join("stderr.txt");
        std::fs::write(&stdout_path, &outcome.stdout)?;
        std::fs::write(&stderr_path, &outcome.stderr)?;
        let inventory = inventory_root(&self.root)?;
        let state = capture_state(&self.root, &self.run_dir, &inventory);
        write_json(&directory.join("root-files.json"), &inventory)?;
        write_json(&directory.join("state.json"), &state)?;
        write_json(
            &directory.join("command.json"),
            &CommandEvidence {
                schema_version: EVIDENCE_SCHEMA_VERSION,
                command: &spec,
                outcome: &outcome,
                process_error: &process_error,
            },
        )?;
        Ok(StepArtifacts {
            name: name.to_string(),
            spec,
            outcome,
            directory,
            stdout_path,
            stderr_path,
            process_error,
        })
    }

    fn record_step(
        &mut self,
        step: StepArtifacts,
        assertion_errors: Vec<String>,
    ) -> Result<(), AcceptanceFailure> {
        let passed = assertion_errors.is_empty();
        self.steps.push(StepSummary {
            step: step.name.clone(),
            passed,
            exit_code: step.outcome.exit_code,
            timed_out: step.outcome.timed_out,
            assertion_errors: assertion_errors.clone(),
            evidence_directory: step.directory.to_string_lossy().into_owned(),
        });
        if passed {
            return Ok(());
        }

        let failure = AcceptanceFailure::new(
            step.name,
            step.spec.arguments,
            step.outcome.exit_code,
            step.outcome.timed_out,
            assertion_errors,
            step.stdout_path.to_string_lossy().into_owned(),
            step.stderr_path.to_string_lossy().into_owned(),
            self.evidence.to_string_lossy().into_owned(),
        );
        let remaining = classify_package_dirs(&self.root).unwrap_or_default();
        let summary = self.summary("failed", remaining);
        let _ = self.finalize(&summary);
        Err(failure)
    }

    fn preflight_failure(&mut self, assertion_errors: Vec<String>) -> AcceptanceFailure {
        let failure = AcceptanceFailure::new(
            StepName::Preflight.as_str().to_string(),
            Vec::new(),
            None,
            false,
            assertion_errors.clone(),
            String::new(),
            String::new(),
            self.evidence.to_string_lossy().into_owned(),
        );
        self.steps.push(StepSummary {
            step: StepName::Preflight.as_str().to_string(),
            passed: false,
            exit_code: None,
            timed_out: false,
            assertion_errors,
            evidence_directory: self.evidence.to_string_lossy().into_owned(),
        });
        let summary = self.summary("failed", Vec::new());
        let _ = self.finalize(&summary);
        failure
    }

    fn summary(
        &self,
        status: &str,
        remaining_payloads: Vec<super::model::PackageDirectoryEvidence>,
    ) -> RunSummary {
        RunSummary {
            schema_version: EVIDENCE_SCHEMA_VERSION,
            run_id: self.run_id.clone(),
            status: status.to_string(),
            package_id: self.config.package_id.clone(),
            query: self.config.query.clone(),
            resolved_version: self.resolved_version.clone(),
            registry_key_id: self.registry_key_id.clone(),
            registry_index_sha256: self.registry_index_sha256.clone(),
            signing_key_fingerprint: self.signing_key_fingerprint.clone(),
            executable_sha256: self.executable_sha256.clone(),
            doctor_errors: self.doctor_errors,
            doctor_warnings: self.doctor_warnings,
            steps: self.steps.clone(),
            remaining_payloads,
            evidence_directory: self.evidence.to_string_lossy().into_owned(),
        }
    }

    fn validate_mutable_nu_paths(&self, paths: &NuPaths, errors: &mut Vec<String>) {
        let mut mutable_paths = vec![("plugin registry", paths.plugin_registry_path.as_str())];
        if let Some(data_dir) = paths.data_dir.as_deref() {
            mutable_paths.push(("Nu data directory", data_dir));
        } else {
            errors.push("Nu data directory is absent from paths.json".to_string());
        }
        for vendor in &paths.vendor_autoload_dirs {
            mutable_paths.push(("vendor-autoload directory", vendor));
        }
        if let Some(selected) = paths.vendor_autoload_dir.as_deref() {
            mutable_paths.push(("selected vendor-autoload target", selected));
        }
        for (label, path) in mutable_paths {
            match path_is_contained(Path::new(path), &self.home) {
                Ok(true) => {}
                Ok(false) => {
                    errors.push(format!("{label} escaped isolated home directory: {path}"))
                }
                Err(error) => errors.push(format!("could not contain-check {label}: {error}")),
            }
        }
    }
}

pub fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(value)?;
    std::fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))
}

fn basic_errors(step: &StepArtifacts) -> Vec<String> {
    let mut errors = Vec::new();
    if let Some(error) = &step.process_error {
        errors.push(format!("command could not start: {error}"));
    }
    if step.outcome.timed_out {
        errors.push(format!(
            "command exceeded {} ms and was killed",
            step.spec.timeout_ms
        ));
    }
    if step.outcome.exit_code != Some(0) {
        errors.push(format!("command exited with {:?}", step.outcome.exit_code));
    }
    errors
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path, errors: &mut Vec<String>) -> Option<T> {
    match std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))
        .and_then(|content| serde_json::from_str(&content).map_err(Into::into))
    {
        Ok(value) => Some(value),
        Err(error) => {
            errors.push(error.to_string());
            None
        }
    }
}

fn validate_contained_existing(path: &Path, root: &Path, label: &str, errors: &mut Vec<String>) {
    if !path.exists() {
        errors.push(format!("{label} does not exist: {}", path.display()));
        return;
    }
    match path_is_contained(path, root) {
        Ok(true) => {}
        Ok(false) => errors.push(format!("{label} escaped root: {}", path.display())),
        Err(error) => errors.push(format!("could not contain-check {label}: {error}")),
    }
}

fn require_clear_journals(root: &Path, errors: &mut Vec<String>) {
    match discover_journals(root) {
        Ok(journals) if journals.is_empty() => {}
        Ok(journals) => errors.push(format!(
            "pending activation/autoload/lifecycle journals remain: {}",
            journals
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )),
        Err(error) => errors.push(format!("failed to discover journals: {error}")),
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) if cfg!(windows) => left
            .to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy()),
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn render_summary(summary: &RunSummary) -> String {
    let mut output = format!(
        "# Official registry Stage 1 acceptance\n\n- Run: `{}`\n- Status: **{}**\n- Package: `{}`\n- Resolved version: `{}`\n- Registry key: `{}`\n- Doctor: {} error(s), {} warning(s)\n\n## Steps\n",
        summary.run_id,
        summary.status,
        summary.package_id,
        summary.resolved_version.as_deref().unwrap_or("unresolved"),
        summary.registry_key_id.as_deref().unwrap_or("unresolved"),
        summary.doctor_errors.unwrap_or_default(),
        summary.doctor_warnings.unwrap_or_default()
    );
    for step in &summary.steps {
        output.push_str(&format!(
            "\n- `{}`: {} (exit {:?}, timeout {})",
            step.step,
            if step.passed { "passed" } else { "failed" },
            step.exit_code,
            step.timed_out
        ));
        for error in &step.assertion_errors {
            output.push_str(&format!("\n  - {error}"));
        }
    }
    output.push_str("\n\n## Remaining payloads\n");
    if summary.remaining_payloads.is_empty() {
        output.push_str("\n- None\n");
    } else {
        for payload in &summary.remaining_payloads {
            let labels = payload
                .references
                .iter()
                .map(|reference| reference.source.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            output.push_str(&format!(
                "\n- `{}`: {}\n",
                payload.path,
                if payload.orphan { "orphan" } else { &labels }
            ));
        }
    }
    output
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}
