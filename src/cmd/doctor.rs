use anyhow::Result;
use clap::Args;
use console::style;
use serde::Serialize;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use crate::cmd::activate::{execute as activate_execute, ActivateArgs};
use crate::cmd::init::{ensure_official_registry_config, execute as init_execute, InitArgs};
use crate::cmd::registry::{self, RegistryCommands};
use crate::cmd::setup::{self, NuSetupArgs};
use crate::config::Config;
use crate::core::official_registry::OFFICIAL_REGISTRY;
use crate::core::registry::RegistryManager;
use crate::nu::paths::{discover_nu_off_path, find_nu_executable_with_root, NuPaths};
use crate::nupm_compat::NupmCompatibility;
use crate::nupm_compat::{
    count_drifted_imports, resolve_nupm_home, scan_nupm_home, NupmHomeResolution,
};
use crate::state::autoload_journal::PendingAutoload;
use crate::state::autoload_state::AutoloadState;
use crate::state::journal::PendingActivation;
use crate::state::lifecycle_journal::PendingLifecycle;
use crate::state::lockfile::Lockfile;
use crate::state::nupm_import::NupmImportsFile;
use crate::util::fs_safety::{acquire_mutation_lock, assert_managed_file_owned};
use crate::util::hints::{
    self, registry_none_fix, setup_nu_use_existing, CMD_ACTIVATE, CMD_INIT, CMD_INIT_REFRESH,
    CMD_REGISTRY_SYNC, CMD_SETUP_NU,
};

const SCHEMA_VERSION: u32 = 1;
const LAYOUT_DIRS: &[&str] = &["nu_state", "state", "packages", "registries"];

#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Apply safe automated repairs after reporting
    #[arg(long)]
    pub fix: bool,

    /// Skip confirmation prompts for confirm-tier repairs (non-TTY implies --yes)
    #[arg(long)]
    pub yes: bool,

    /// Emit JSON report (no ANSI styling)
    #[arg(long)]
    pub json: bool,

    /// Override nupm home for coexistence checks
    #[arg(long)]
    pub nupm_home: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Ok,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairTier {
    None,
    Auto,
    Confirm,
    Manual,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub id: String,
    pub severity: Severity,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
    pub repair: RepairTier,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RepairStatus {
    Applied,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepairRecord {
    pub id: String,
    pub status: RepairStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub schema_version: u32,
    pub root: String,
    pub summary: Summary,
    pub findings: Vec<Finding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repairs: Option<Vec<RepairRecord>>,
}

#[derive(Default)]
pub struct DoctorOptions {
    pub skip_network: bool,
    /// Override init repair (tests inject fakes; production uses `init::execute`).
    pub init_repair: Option<fn(&InitArgs, &Path) -> Result<()>>,
    /// Override activate repair (tests inject fakes; production uses `activate::execute`).
    pub activate_repair: Option<fn(&ActivateArgs, &Path) -> Result<()>>,
    /// Override Nushell bootstrap repair (tests inject fakes; production uses `setup::execute_nu_impl`).
    pub nu_setup_repair: Option<fn(&NuSetupArgs, &Path) -> Result<()>>,
    /// Override off-PATH Nu discovery (tests inject a known binary path).
    pub discover_off_path: Option<fn() -> Option<PathBuf>>,
}

pub fn execute(args: &DoctorArgs, root: &Path) -> Result<i32> {
    execute_with_options(args, root, DoctorOptions::default())
}

pub fn execute_with_options(args: &DoctorArgs, root: &Path, options: DoctorOptions) -> Result<i32> {
    let mut report = run_checks_with_options(args, root, &options)?;
    if args.fix {
        let repairs = apply_repairs(args, root, &report.findings, &options)?;
        report = run_checks_with_options(args, root, &options)?;
        report.repairs = Some(repairs);
    }
    print_report(args, root, &report)?;
    Ok(report.exit_code())
}

impl DoctorReport {
    fn exit_code(&self) -> i32 {
        if self
            .findings
            .iter()
            .any(|f| f.id == "root.writable" && f.severity == Severity::Error)
        {
            return 2;
        }
        if self.summary.errors > 0 {
            return 1;
        }
        0
    }
}

fn finding(
    id: &str,
    severity: Severity,
    message: impl Into<String>,
    fix: Option<&str>,
    repair: RepairTier,
) -> Finding {
    Finding {
        id: id.to_string(),
        severity,
        message: message.into(),
        fix: fix.map(str::to_string),
        repair,
    }
}

pub fn run_checks(args: &DoctorArgs, root: &Path) -> Result<DoctorReport> {
    run_checks_with_options(args, root, &DoctorOptions::default())
}

pub fn run_checks_with_options(
    args: &DoctorArgs,
    root: &Path,
    options: &DoctorOptions,
) -> Result<DoctorReport> {
    let mut findings = Vec::new();

    check_root_layout(root, &mut findings);
    let nu_paths = check_nu_paths(root, options, &mut findings);
    check_journals(root, nu_paths.as_ref(), &mut findings);
    let lockfile = check_lockfile(root, nu_paths.as_ref(), &mut findings);
    if let (Some(paths), Some(lf)) = (nu_paths.as_ref(), lockfile.as_ref()) {
        check_activation(root, paths, lf, &mut findings);
    }
    if let Some(lf) = lockfile.as_ref() {
        check_payloads(root, lf, &mut findings);
    }
    check_registry(root, &mut findings);
    if Config::load(root)?.nupm_compat.scan_on_doctor {
        check_nupm(args, root, lockfile.as_ref(), &mut findings);
    }

    Ok(DoctorReport {
        schema_version: SCHEMA_VERSION,
        root: root.display().to_string(),
        summary: summarize(&findings),
        findings,
        repairs: None,
    })
}

fn summarize(findings: &[Finding]) -> Summary {
    Summary {
        errors: findings
            .iter()
            .filter(|f| f.severity == Severity::Error)
            .count(),
        warnings: findings
            .iter()
            .filter(|f| f.severity == Severity::Warn)
            .count(),
        infos: findings
            .iter()
            .filter(|f| f.severity == Severity::Info)
            .count(),
    }
}

fn check_root_layout(root: &Path, findings: &mut Vec<Finding>) {
    if !root.exists() {
        findings.push(finding(
            "root.writable",
            Severity::Error,
            format!("Numan root '{}' does not exist.", root.display()),
            None,
            RepairTier::Manual,
        ));
        return;
    }

    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join(".doctor-write-test"))
    {
        Ok(_) => {
            let _ = std::fs::remove_file(root.join(".doctor-write-test"));
            findings.push(finding(
                "root.writable",
                Severity::Ok,
                "Numan root is writable".to_string(),
                None,
                RepairTier::None,
            ));
        }
        Err(e) => {
            findings.push(finding(
                "root.writable",
                Severity::Error,
                format!("Numan root is not writable: {e}"),
                None,
                RepairTier::Manual,
            ));
        }
    }

    for dir in LAYOUT_DIRS {
        let id = format!("layout.{dir}");
        if root.join(dir).is_dir() {
            findings.push(finding(
                &id,
                Severity::Ok,
                format!("'{dir}/' present"),
                None,
                RepairTier::None,
            ));
        } else {
            findings.push(finding(
                &id,
                Severity::Warn,
                format!("Missing layout directory '{dir}/'"),
                None,
                RepairTier::Auto,
            ));
        }
    }
}

fn resolve_off_path(options: &DoctorOptions) -> Option<PathBuf> {
    if let Some(discover) = options.discover_off_path {
        discover()
    } else {
        discover_nu_off_path()
    }
}

fn nu_is_available(root: &Path) -> bool {
    if find_nu_executable_with_root(root).is_ok() {
        return true;
    }
    if let Ok(paths) = NuPaths::load(root) {
        let exe = Path::new(&paths.nu_executable);
        if exe.is_file() && paths.validate_drift().is_ok() {
            return true;
        }
    }
    false
}

fn check_nu_paths(
    root: &Path,
    options: &DoctorOptions,
    findings: &mut Vec<Finding>,
) -> Option<NuPaths> {
    let nu_available = nu_is_available(root);
    if !nu_available {
        if let Some(off_path) = resolve_off_path(options) {
            let fix_hint = setup_nu_use_existing(&off_path);
            findings.push(finding(
                "nu.binary.found_off_path",
                Severity::Warn,
                format!("Nushell found at '{}' but not on PATH.", off_path.display()),
                Some(&fix_hint),
                RepairTier::Confirm,
            ));
            findings.push(finding(
                "nu.binary.missing_on_path",
                Severity::Ok,
                "Nushell is installed off PATH (see nu.binary.found_off_path)",
                None,
                RepairTier::None,
            ));
        } else {
            findings.push(finding(
                "nu.binary.missing_on_path",
                Severity::Error,
                "Nu not found on PATH or in the Numan tools directory.",
                Some(CMD_SETUP_NU),
                RepairTier::Confirm,
            ));
        }
    } else {
        findings.push(finding(
            "nu.binary.missing_on_path",
            Severity::Ok,
            "Nushell binary is available",
            None,
            RepairTier::None,
        ));
    }

    let paths_path = root.join("nu_state/paths.json");
    if !paths_path.exists() {
        findings.push(finding(
            "nu_paths.missing",
            Severity::Error,
            "Nu paths are not cached (not initialized)",
            Some(CMD_INIT),
            RepairTier::Auto,
        ));
        return None;
    }

    let paths = match NuPaths::load(root) {
        Ok(p) => p,
        Err(e) => {
            findings.push(finding(
                "nu_paths.parse",
                Severity::Error,
                format!("Failed to read Nu paths: {e}"),
                Some(CMD_INIT),
                RepairTier::Manual,
            ));
            return None;
        }
    };

    match paths.validate_drift() {
        Ok(()) => findings.push(finding(
            "nu_paths.drift",
            Severity::Ok,
            format!("Nu binary hash matches ({})", paths.nu_version),
            None,
            RepairTier::None,
        )),
        Err(e) => findings.push(finding(
            "nu_paths.drift",
            Severity::Error,
            e.to_string(),
            Some(CMD_INIT_REFRESH),
            RepairTier::Confirm,
        )),
    }

    if paths.data_dir.is_some() && nu_available {
        match NuPaths::detect_with_root(root)
            .and_then(|live| paths.validate_vendor_drift(&live.vendor_autoload_dirs))
        {
            Ok(()) => findings.push(finding(
                "nu_paths.vendor_drift",
                Severity::Ok,
                "Vendor-autoload target matches cached Nu environment",
                None,
                RepairTier::None,
            )),
            Err(e) => findings.push(finding(
                "nu_paths.vendor_drift",
                Severity::Error,
                e.to_string(),
                Some(CMD_INIT_REFRESH),
                RepairTier::Confirm,
            )),
        }
    }

    Some(paths)
}

fn check_journals(root: &Path, nu_paths: Option<&NuPaths>, findings: &mut Vec<Finding>) {
    if let Ok(Some(j)) = PendingActivation::load(root) {
        if let Some(paths) = nu_paths {
            if !j.matches_nu_identity(
                &paths.nu_executable_hash,
                &paths.nu_version,
                &paths.plugin_registry_path,
            ) {
                findings.push(finding(
                    "journal.plugin_stale",
                    Severity::Error,
                    "Pending plugin activation journal has stale Nu identity",
                    Some(CMD_INIT_REFRESH),
                    RepairTier::Confirm,
                ));
            } else {
                findings.push(finding(
                    "journal.plugin_pending",
                    Severity::Warn,
                    "Pending plugin activation journal detected",
                    Some(CMD_ACTIVATE),
                    RepairTier::Confirm,
                ));
            }
        } else {
            findings.push(finding(
                "journal.plugin_pending",
                Severity::Warn,
                "Pending plugin activation journal detected",
                Some(CMD_ACTIVATE),
                RepairTier::Confirm,
            ));
        }
    }

    if let Ok(Some(j)) = PendingAutoload::load(root) {
        if let Some(paths) = nu_paths {
            if !j.matches_nu_identity(&paths.nu_executable_hash, &paths.nu_version) {
                findings.push(finding(
                    "journal.autoload_stale",
                    Severity::Error,
                    "Pending module-autoload journal has stale Nu identity",
                    Some(CMD_INIT_REFRESH),
                    RepairTier::Confirm,
                ));
            } else {
                findings.push(finding(
                    "journal.autoload_pending",
                    Severity::Warn,
                    "Pending module-autoload journal detected",
                    Some(CMD_ACTIVATE),
                    RepairTier::Confirm,
                ));
            }
        } else {
            findings.push(finding(
                "journal.autoload_pending",
                Severity::Warn,
                "Pending module-autoload journal detected",
                Some(CMD_ACTIVATE),
                RepairTier::Confirm,
            ));
        }
    }

    if let Ok(Some(j)) = PendingLifecycle::load(root) {
        findings.push(finding(
            "journal.lifecycle_pending",
            Severity::Warn,
            format!(
                "Pending lifecycle journal (op: {:?}, stage: {:?}, package: {})",
                j.op, j.stage, j.package_id
            ),
            None,
            RepairTier::Manual,
        ));
        findings.push(finding(
            "journal.lifecycle_stale",
            Severity::Error,
            "Interrupted lifecycle operation requires manual recovery",
            None,
            RepairTier::Manual,
        ));
    }
}

fn check_lockfile(
    root: &Path,
    nu_paths: Option<&NuPaths>,
    findings: &mut Vec<Finding>,
) -> Option<Lockfile> {
    let lock_path = root.join("lockfile");
    if !lock_path.exists() {
        findings.push(finding(
            "lockfile.missing",
            Severity::Info,
            "No packages installed",
            None,
            RepairTier::None,
        ));
        return Some(Lockfile::empty());
    }

    let content = match std::fs::read_to_string(&lock_path) {
        Ok(c) => c,
        Err(e) => {
            findings.push(finding(
                "lockfile.parse",
                Severity::Error,
                format!("Cannot read lockfile: {e}"),
                None,
                RepairTier::Manual,
            ));
            return None;
        }
    };

    let lockfile: Lockfile = match serde_json::from_str(&content) {
        Ok(lf) => lf,
        Err(e) => {
            findings.push(finding(
                "lockfile.parse",
                Severity::Error,
                format!("Lockfile JSON is invalid: {e}"),
                None,
                RepairTier::Manual,
            ));
            return None;
        }
    };

    if lockfile.is_empty() {
        findings.push(finding(
            "lockfile.missing",
            Severity::Info,
            "No packages installed",
            None,
            RepairTier::None,
        ));
    }

    if let Some(paths) = nu_paths {
        let has_active_modules = lockfile
            .packages
            .values()
            .any(|e| e.module_activation.is_some());
        if has_active_modules && paths.vendor_autoload_dir.is_none() {
            findings.push(finding(
                "nu_paths.vendor_missing",
                Severity::Warn,
                "Active modules require a Numan-safe vendor-autoload directory",
                Some(CMD_INIT_REFRESH),
                RepairTier::Manual,
            ));
        }
    }

    Some(lockfile)
}

fn check_activation(
    root: &Path,
    paths: &NuPaths,
    lockfile: &Lockfile,
    findings: &mut Vec<Finding>,
) {
    let vendor_dir = paths.vendor_autoload_dir.as_deref().unwrap_or("");
    let managed_path = if vendor_dir.is_empty() {
        String::new()
    } else {
        format!("{vendor_dir}/numan.nu")
    };

    for (id, entry) in &lockfile.packages {
        if entry.package_type == "plugin"
            && entry.activation.is_some()
            && !entry.is_active_for(
                &paths.nu_executable_hash,
                &paths.nu_version,
                &paths.plugin_registry_path,
            )
        {
            findings.push(finding(
                "activation.plugin_stale",
                Severity::Warn,
                format!("Plugin '{id}' activation is stale for current Nu"),
                Some(CMD_ACTIVATE),
                RepairTier::Confirm,
            ));
        }
        if entry.package_type == "module"
            && entry.module_activation.is_some()
            && !entry.is_module_active_for(
                &paths.nu_executable_hash,
                &paths.nu_version,
                vendor_dir,
                &managed_path,
            )
        {
            findings.push(finding(
                "activation.module_stale",
                Severity::Warn,
                format!("Module '{id}' activation is stale for current Nu"),
                Some(CMD_ACTIVATE),
                RepairTier::Confirm,
            ));
        }
    }

    if let Ok(Some(state)) = AutoloadState::load(root) {
        if let Err(e) = state.validate_against_lockfile(lockfile) {
            findings.push(finding(
                "autoload.projection",
                Severity::Error,
                e.to_string(),
                Some(CMD_ACTIVATE),
                RepairTier::Confirm,
            ));
        }
    }

    let has_active_modules = lockfile
        .packages
        .values()
        .any(|e| e.module_activation.is_some());
    if has_active_modules && !managed_path.is_empty() {
        let managed = Path::new(&managed_path);
        if !managed.is_file() {
            findings.push(finding(
                "autoload.managed_missing",
                Severity::Warn,
                format!("Managed autoload file '{}' is missing", managed.display()),
                Some(CMD_ACTIVATE),
                RepairTier::Confirm,
            ));
        } else if let Err(e) = assert_managed_file_owned(managed) {
            findings.push(finding(
                "autoload.managed_foreign",
                Severity::Error,
                e.to_string(),
                None,
                RepairTier::Manual,
            ));
        }
    }
}

fn check_payloads(root: &Path, lockfile: &Lockfile, findings: &mut Vec<Finding>) {
    for (id, entry) in &lockfile.packages {
        let payload = root.join(&entry.payload_path);
        if !payload.exists() {
            findings.push(finding(
                "payload.missing",
                Severity::Error,
                format!("Payload missing for '{id}' at '{}'", entry.payload_path),
                Some(&hints::install_pkg(id)),
                RepairTier::Manual,
            ));
        }
    }
}

fn check_registry(root: &Path, findings: &mut Vec<Finding>) {
    let config = match Config::load(root) {
        Ok(c) => c,
        Err(e) => {
            findings.push(finding(
                "registry.config",
                Severity::Error,
                format!("Cannot read config.toml: {e}"),
                None,
                RepairTier::Manual,
            ));
            return;
        }
    };

    if config.registries.is_empty() {
        let repair = if OFFICIAL_REGISTRY.is_placeholder_key() {
            RepairTier::Manual
        } else {
            RepairTier::Auto
        };
        findings.push(finding(
            "registry.none",
            Severity::Warn,
            "No registries configured",
            Some(registry_none_fix(root)),
            repair,
        ));
        return;
    }

    let mgr = match RegistryManager::new(root) {
        Ok(m) => m,
        Err(e) => {
            findings.push(finding(
                "registry.trust",
                Severity::Error,
                format!("Cannot load trust store: {e}"),
                None,
                RepairTier::Manual,
            ));
            return;
        }
    };

    for (name, reg) in &config.registries {
        if !reg.enabled {
            continue;
        }
        if !mgr.index_path(name).exists() {
            findings.push(finding(
                "registry.index_missing",
                Severity::Info,
                format!("Registry '{name}' index is not cached"),
                Some(CMD_REGISTRY_SYNC),
                RepairTier::Auto,
            ));
        }
    }
}

fn check_nupm(
    args: &DoctorArgs,
    root: &Path,
    lockfile: Option<&Lockfile>,
    findings: &mut Vec<Finding>,
) {
    let drift_count = count_drifted_imports(root).unwrap_or(0);
    if drift_count > 0 {
        findings.push(finding(
            "nupm.drift",
            Severity::Warn,
            format!("{drift_count} nupm import(s) have source drift"),
            Some(CMD_NUPM_DIFF_PLACEHOLDER),
            RepairTier::Manual,
        ));
    }

    match resolve_nupm_home(args.nupm_home.as_deref()) {
        Ok(NupmHomeResolution::NotConfigured) => {
            findings.push(finding(
                "nupm.home_unconfigured",
                Severity::Info,
                "nupm home not configured (pass --nupm-home or set NUPM_HOME)",
                None,
                RepairTier::None,
            ));
        }
        Ok(NupmHomeResolution::Found(home)) => {
            if let Ok(scan) = scan_nupm_home(&home) {
                if let Some(lf) = lockfile {
                    if let Ok(overlap) = count_nupm_name_overlap(root, lf, &scan.source_roots) {
                        if overlap > 0 {
                            findings.push(finding(
                                "nupm.overlap",
                                Severity::Info,
                                format!("{overlap} potential nupm name overlap(s) with lockfile"),
                                None,
                                RepairTier::None,
                            ));
                        }
                    }
                }
            }
        }
        Err(e) => {
            findings.push(finding(
                "nupm.scan_failed",
                Severity::Warn,
                format!("nupm discovery failed: {e}"),
                None,
                RepairTier::Manual,
            ));
        }
    }
}

const CMD_NUPM_DIFF_PLACEHOLDER: &str = "numan nupm diff <owner/name>";

fn count_nupm_name_overlap(
    root: &Path,
    lockfile: &Lockfile,
    source_roots: &[crate::nupm_compat::SourceRootEntry],
) -> Result<usize> {
    let imports = NupmImportsFile::load(root)?;
    let mut count = 0usize;
    for entry in source_roots {
        if entry.compatibility != NupmCompatibility::ImportableModule {
            continue;
        }
        let Some(meta) = &entry.metadata else {
            continue;
        };
        for (installed_id, lf_entry) in &lockfile.packages {
            if lf_entry.package_type != "module" {
                continue;
            }
            let Some((_, name)) = installed_id.split_once('/') else {
                continue;
            };
            if name != meta.name {
                continue;
            }
            let same_import = imports
                .imports
                .get(installed_id.as_str())
                .is_some_and(|r| Path::new(&r.nupm_source_path) == entry.source_path.as_path());
            if !same_import {
                count += 1;
                break;
            }
        }
    }
    Ok(count)
}

fn confirm_repairs(args: &DoctorArgs) -> bool {
    args.yes || !std::io::stdin().is_terminal()
}

fn apply_repairs(
    args: &DoctorArgs,
    root: &Path,
    findings: &[Finding],
    options: &DoctorOptions,
) -> Result<Vec<RepairRecord>> {
    let needs_lock = findings.iter().any(|f| {
        matches!(f.repair, RepairTier::Auto | RepairTier::Confirm) && f.severity != Severity::Ok
    });
    let _lock = if needs_lock {
        Some(acquire_mutation_lock(root)?)
    } else {
        None
    };

    let mut records = Vec::new();
    let confirm = confirm_repairs(args);

    for dir in LAYOUT_DIRS {
        let id = format!("layout.{dir}");
        if findings
            .iter()
            .any(|f| f.id == id && f.severity == Severity::Warn)
        {
            match std::fs::create_dir_all(root.join(dir)) {
                Ok(()) => records.push(RepairRecord {
                    id,
                    status: RepairStatus::Applied,
                    reason: None,
                }),
                Err(e) => records.push(RepairRecord {
                    id,
                    status: RepairStatus::Failed,
                    reason: Some(e.to_string()),
                }),
            }
        }
    }

    if findings
        .iter()
        .any(|f| f.id == "nu.binary.found_off_path" && f.severity == Severity::Warn)
    {
        let id = "nu.binary.found_off_path".to_string();
        if !confirm {
            records.push(RepairRecord {
                id,
                status: RepairStatus::Skipped,
                reason: Some("not_confirmed".to_string()),
            });
        } else if let Some(off_path) = resolve_off_path(options) {
            let setup_fn = options.nu_setup_repair.unwrap_or(setup::execute_nu_impl);
            match setup_fn(
                &NuSetupArgs {
                    force: false,
                    skip_path: false,
                    yes: true,
                    version: None,
                    use_existing: Some(off_path),
                },
                root,
            ) {
                Ok(()) => records.push(RepairRecord {
                    id,
                    status: RepairStatus::Applied,
                    reason: None,
                }),
                Err(e) => records.push(RepairRecord {
                    id,
                    status: RepairStatus::Failed,
                    reason: Some(e.to_string()),
                }),
            }
        } else {
            records.push(RepairRecord {
                id,
                status: RepairStatus::Skipped,
                reason: Some("off_path_not_found".to_string()),
            });
        }
    }

    if findings
        .iter()
        .any(|f| f.id == "nu.binary.missing_on_path" && f.severity == Severity::Error)
    {
        let id = "nu.binary.missing_on_path".to_string();
        if !confirm {
            records.push(RepairRecord {
                id,
                status: RepairStatus::Skipped,
                reason: Some("not_confirmed".to_string()),
            });
        } else if options.skip_network {
            records.push(RepairRecord {
                id,
                status: RepairStatus::Skipped,
                reason: Some("skip_network".to_string()),
            });
        } else {
            let setup_fn = options.nu_setup_repair.unwrap_or(setup::execute_nu_impl);
            match setup_fn(
                &NuSetupArgs {
                    force: false,
                    skip_path: false,
                    yes: true,
                    version: None,
                    use_existing: None,
                },
                root,
            ) {
                Ok(()) => records.push(RepairRecord {
                    id,
                    status: RepairStatus::Applied,
                    reason: None,
                }),
                Err(e) => records.push(RepairRecord {
                    id,
                    status: RepairStatus::Failed,
                    reason: Some(e.to_string()),
                }),
            }
        }
    }

    if findings
        .iter()
        .any(|f| f.id == "nu_paths.missing" && f.severity == Severity::Error)
    {
        let id = "nu_paths.missing".to_string();
        let init_fn = options.init_repair.unwrap_or(init_execute);
        match init_fn(&InitArgs { refresh: false }, root) {
            Ok(()) => records.push(RepairRecord {
                id,
                status: RepairStatus::Applied,
                reason: None,
            }),
            Err(e) => records.push(RepairRecord {
                id,
                status: RepairStatus::Failed,
                reason: Some(e.to_string()),
            }),
        }
    }

    if findings.iter().any(|f| {
        f.id == "registry.index_missing" && f.severity == Severity::Info && !options.skip_network
    }) {
        let id = "registry.index_missing".to_string();
        match registry::execute(RegistryCommands::Sync, root) {
            Ok(()) => records.push(RepairRecord {
                id,
                status: RepairStatus::Applied,
                reason: None,
            }),
            Err(e) => records.push(RepairRecord {
                id,
                status: RepairStatus::Failed,
                reason: Some(e.to_string()),
            }),
        }
    } else if findings.iter().any(|f| f.id == "registry.index_missing") && options.skip_network {
        records.push(RepairRecord {
            id: "registry.index_missing".to_string(),
            status: RepairStatus::Skipped,
            reason: Some("skip_network".to_string()),
        });
    }

    if findings
        .iter()
        .any(|f| f.id == "registry.none" && f.repair == RepairTier::Auto)
    {
        let id = "registry.none".to_string();
        match Config::load(root) {
            Ok(mut config) => match ensure_official_registry_config(root, &mut config) {
                Ok(true) => records.push(RepairRecord {
                    id,
                    status: RepairStatus::Applied,
                    reason: None,
                }),
                Ok(false) => records.push(RepairRecord {
                    id,
                    status: RepairStatus::Skipped,
                    reason: Some("official registry already configured".to_string()),
                }),
                Err(e) => records.push(RepairRecord {
                    id,
                    status: RepairStatus::Failed,
                    reason: Some(e.to_string()),
                }),
            },
            Err(e) => records.push(RepairRecord {
                id,
                status: RepairStatus::Failed,
                reason: Some(e.to_string()),
            }),
        }
    }

    let needs_refresh = findings.iter().any(|f| {
        matches!(f.id.as_str(), "nu_paths.drift" | "nu_paths.vendor_drift")
            && f.severity == Severity::Error
    }) || findings.iter().any(|f| {
        matches!(
            f.id.as_str(),
            "journal.plugin_stale" | "journal.autoload_stale"
        ) && f.severity == Severity::Error
    });

    if needs_refresh {
        let id = "nu_paths.refresh".to_string();
        if !confirm {
            records.push(RepairRecord {
                id,
                status: RepairStatus::Skipped,
                reason: Some("not_confirmed".to_string()),
            });
        } else {
            let init_fn = options.init_repair.unwrap_or(init_execute);
            match init_fn(&InitArgs { refresh: true }, root) {
                Ok(()) => records.push(RepairRecord {
                    id,
                    status: RepairStatus::Applied,
                    reason: None,
                }),
                Err(e) => records.push(RepairRecord {
                    id,
                    status: RepairStatus::Failed,
                    reason: Some(e.to_string()),
                }),
            }
        }
    }

    let needs_activate = findings.iter().any(|f| {
        f.repair == RepairTier::Confirm
            && matches!(
                f.id.as_str(),
                "journal.plugin_pending"
                    | "journal.autoload_pending"
                    | "activation.plugin_stale"
                    | "activation.module_stale"
                    | "autoload.projection"
                    | "autoload.managed_missing"
            )
            && f.severity != Severity::Ok
    });

    if needs_activate {
        let id = "activation.reconcile".to_string();
        if !confirm {
            records.push(RepairRecord {
                id,
                status: RepairStatus::Skipped,
                reason: Some("not_confirmed".to_string()),
            });
        } else {
            let activate_args = ActivateArgs {
                packages: Vec::new(),
                yes: true,
                verbose: false,
                list: false,
                check: false,
            };
            let activate_fn = options.activate_repair.unwrap_or(activate_execute);
            match activate_fn(&activate_args, root) {
                Ok(()) => records.push(RepairRecord {
                    id,
                    status: RepairStatus::Applied,
                    reason: None,
                }),
                Err(e) => records.push(RepairRecord {
                    id,
                    status: RepairStatus::Failed,
                    reason: Some(e.to_string()),
                }),
            }
        }
    }

    Ok(records)
}

fn print_report(args: &DoctorArgs, root: &Path, report: &DoctorReport) -> Result<()> {
    if args.json {
        let json = serde_json::to_string_pretty(report)?;
        println!("{json}");
        return Ok(());
    }

    let mut out = std::io::stdout();
    writeln!(out, "Numan doctor — {}", root.display())?;
    writeln!(out)?;

    let sections: &[(&str, &[&str])] = &[
        (
            "Root",
            &[
                "root.writable",
                "layout.nu_state",
                "layout.state",
                "layout.packages",
                "layout.registries",
            ],
        ),
        (
            "Initialization",
            &[
                "nu.binary.missing_on_path",
                "nu.binary.found_off_path",
                "nu_paths.missing",
                "nu_paths.drift",
                "nu_paths.vendor_drift",
                "nu_paths.vendor_missing",
            ],
        ),
        (
            "Journals",
            &[
                "journal.plugin_pending",
                "journal.plugin_stale",
                "journal.autoload_pending",
                "journal.autoload_stale",
                "journal.lifecycle_pending",
                "journal.lifecycle_stale",
            ],
        ),
        (
            "Activation",
            &[
                "lockfile.missing",
                "lockfile.parse",
                "activation.plugin_stale",
                "activation.module_stale",
                "autoload.projection",
                "autoload.managed_missing",
                "autoload.managed_foreign",
                "payload.missing",
            ],
        ),
        (
            "Registry",
            &[
                "registry.none",
                "registry.index_missing",
                "registry.config",
                "registry.trust",
            ],
        ),
        (
            "nupm coexistence",
            &[
                "nupm.drift",
                "nupm.home_unconfigured",
                "nupm.overlap",
                "nupm.scan_failed",
            ],
        ),
    ];

    for (title, ids) in sections {
        let section_findings: Vec<_> = report
            .findings
            .iter()
            .filter(|f| ids.contains(&f.id.as_str()) && f.severity != Severity::Ok)
            .collect();
        if section_findings.is_empty() {
            continue;
        }
        writeln!(out, "{title}")?;
        for f in section_findings {
            print_finding(&mut out, f)?;
        }
        writeln!(out)?;
    }

    writeln!(
        out,
        "Summary: {} error(s), {} warning(s)",
        report.summary.errors, report.summary.warnings
    )?;

    if let Some(repairs) = &report.repairs {
        let applied = repairs
            .iter()
            .filter(|r| r.status == RepairStatus::Applied)
            .count();
        let skipped = repairs
            .iter()
            .filter(|r| r.status == RepairStatus::Skipped)
            .count();
        if !repairs.is_empty() {
            writeln!(out)?;
            writeln!(out, "Repairs: {applied} applied, {skipped} skipped")?;
            if skipped > 0 && !args.yes {
                writeln!(out, "(use --yes to apply confirm-tier fixes)")?;
            }
        }
    }

    Ok(())
}

fn print_finding(out: &mut impl Write, f: &Finding) -> Result<()> {
    let symbol = match f.severity {
        Severity::Error => style("✗").red().to_string(),
        Severity::Warn => style("⚠").yellow().to_string(),
        Severity::Info => style("·").dim().to_string(),
        Severity::Ok => style("✓").green().to_string(),
    };
    writeln!(out, "  {symbol} {}", f.message)?;
    if let Some(fix) = &f.fix {
        writeln!(out, "    Fix: {fix}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::init::{execute_with_runner, InitArgs};
    use crate::core::integrity;
    use crate::nu::autoload::FakeCandidateRunner;
    use crate::state::lockfile::{LockfileEntry, PluginActivation};
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    fn fake_paths(root: &Path, nu_exe: &Path) -> NuPaths {
        let bytes = std::fs::read(nu_exe).unwrap();
        NuPaths {
            nu_executable: nu_exe.to_string_lossy().into_owned(),
            nu_version: "0.113.1".to_string(),
            plugin_registry_path: root.join("plugins.msgpackz").to_string_lossy().into_owned(),
            nu_executable_hash: integrity::compute_sha256(&bytes),
            platform: "test".to_string(),
            data_dir: None,
            vendor_autoload_dirs: vec![],
            vendor_autoload_dir: None,
        }
    }

    #[test]
    fn doctor_reports_missing_init() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root).unwrap();
        let args = DoctorArgs {
            fix: false,
            yes: false,
            json: false,
            nupm_home: None,
        };
        let report = run_checks(&args, root).unwrap();
        assert!(report
            .findings
            .iter()
            .any(|f| f.id == "nu_paths.missing" && f.severity == Severity::Error));
    }

    #[test]
    fn doctor_report_only_does_not_create_paths() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root).unwrap();
        let args = DoctorArgs {
            fix: false,
            yes: false,
            json: false,
            nupm_home: None,
        };
        execute_with_options(&args, root, DoctorOptions::default()).unwrap();
        assert!(!root.join("nu_state/paths.json").exists());
    }

    fn fake_runner_factory(_exe: &str) -> Box<dyn crate::nu::autoload::CandidateRunner> {
        Box::new(FakeCandidateRunner::success())
    }

    use crate::nu::bootstrap::managed_nu_binary;

    fn ensure_fake_managed_nu(root: &Path) -> PathBuf {
        let binary = managed_nu_binary(root);
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, b"nu").unwrap();
        binary
    }

    fn test_init_repair(args: &InitArgs, root: &Path) -> Result<()> {
        let nu_exe = ensure_fake_managed_nu(root);
        execute_with_runner(
            args,
            root,
            || Ok(fake_paths(root, &nu_exe)),
            fake_runner_factory,
        )
    }

    #[test]
    fn doctor_fix_auto_creates_layout_and_inits() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root).unwrap();
        std::env::set_var("NUMAN_ROOT", root);
        ensure_fake_managed_nu(root);

        let args = DoctorArgs {
            fix: true,
            yes: true,
            json: false,
            nupm_home: None,
        };
        let code = execute_with_options(
            &args,
            root,
            DoctorOptions {
                skip_network: true,
                init_repair: Some(test_init_repair),
                activate_repair: None,
                nu_setup_repair: None,
                discover_off_path: None,
            },
        )
        .unwrap();
        assert_eq!(code, 0);
        assert!(root.join("nu_state").is_dir());
        assert!(root.join("nu_state/paths.json").is_file());
    }

    #[test]
    fn doctor_fix_adds_official_registry_when_initialized_without_registries() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("nu_state")).unwrap();
        std::env::set_var("NUMAN_ROOT", root);
        let nu_exe = ensure_fake_managed_nu(root);
        fake_paths(root, &nu_exe).save(root).unwrap();
        crate::config::Config::default().save(root).unwrap();

        let args = DoctorArgs {
            fix: true,
            yes: true,
            json: false,
            nupm_home: None,
        };
        let report = run_checks(
            &DoctorArgs {
                fix: false,
                yes: false,
                json: false,
                nupm_home: None,
            },
            root,
        )
        .unwrap();
        let none = report
            .findings
            .iter()
            .find(|f| f.id == "registry.none")
            .expect("registry.none finding");
        if OFFICIAL_REGISTRY.is_placeholder_key() {
            assert_eq!(none.fix.as_deref(), Some(hints::CMD_REGISTRY_ADD));
            assert_eq!(none.repair, RepairTier::Manual);
            return;
        }
        assert_eq!(none.fix.as_deref(), Some(hints::CMD_DOCTOR_FIX));
        assert_eq!(none.repair, RepairTier::Auto);

        execute_with_options(
            &args,
            root,
            DoctorOptions {
                skip_network: true,
                init_repair: None,
                activate_repair: None,
                nu_setup_repair: None,
                discover_off_path: None,
            },
        )
        .unwrap();

        let config = crate::config::Config::load(root).unwrap();
        assert!(config.registries.contains_key(OFFICIAL_REGISTRY.name));
    }

    #[test]
    fn doctor_registry_none_hints_init_before_first_init() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root).unwrap();
        crate::config::Config::default().save(root).unwrap();

        let report = run_checks(
            &DoctorArgs {
                fix: false,
                yes: false,
                json: false,
                nupm_home: None,
            },
            root,
        )
        .unwrap();
        let none = report
            .findings
            .iter()
            .find(|f| f.id == "registry.none")
            .expect("registry.none finding");
        if OFFICIAL_REGISTRY.is_placeholder_key() {
            assert_eq!(none.fix.as_deref(), Some(hints::CMD_REGISTRY_ADD));
        } else {
            assert_eq!(none.fix.as_deref(), Some(CMD_INIT));
        }
    }

    #[test]
    fn doctor_json_output_has_schema() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let nu_exe = root.join("nu");
        std::fs::write(&nu_exe, b"v1").unwrap();
        fake_paths(root, &nu_exe).save(root).unwrap();

        let args = DoctorArgs {
            fix: false,
            yes: false,
            json: true,
            nupm_home: None,
        };
        let report = run_checks(&args, root).unwrap();
        assert_eq!(report.schema_version, 1);
        assert!(report.findings.iter().any(|f| f.id == "nu_paths.drift"));
    }

    #[test]
    fn doctor_detects_stale_plugin_activation() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let nu_v1 = root.join("nu_v1");
        std::fs::write(&nu_v1, b"v1").unwrap();
        let paths = fake_paths(root, &nu_v1);
        paths.save(root).unwrap();

        let mut lockfile = Lockfile::empty();
        lockfile.packages.insert(
            "owner/plugin".to_string(),
            LockfileEntry {
                version: "1.0.0".to_string(),
                package_type: "plugin".to_string(),
                source: "binary".to_string(),
                target: None,
                artifact_url: None,
                artifact_sha256: None,
                executable_path: Some("nu_plugin_test".to_string()),
                archive_root: None,
                include: None,
                entry: None,
                installed_at: "now".to_string(),
                nu_version_at_install: None,
                activation: Some(PluginActivation {
                    plugin_registry_path: "/other/plugins.msgpackz".to_string(),
                    nu_executable_sha256: "wrong".to_string(),
                    nu_version: "0.113.1".to_string(),
                    activated_at: "now".to_string(),
                }),
                registry_url: None,
                registry_revision: None,
                index_sha256: None,
                signing_key_fingerprint: None,
                git_url: None,
                git_rev: None,
                cargo_name: None,
                cargo_lock_sha256: None,
                built_sha256: None,
                payload_path: "packages/plugins/owner/plugin/1.0.0-abc".to_string(),
                revision_id: None,
                payload_sha256: None,
                executable_sha256: None,
                selection_reason: None,
                origin: None,
                module_activation: None,
                module_import_mode: None,
                locked_dependencies: BTreeMap::new(),
            },
        );
        lockfile.save(root).unwrap();
        std::fs::create_dir_all(root.join("packages/plugins/owner/plugin/1.0.0-abc")).unwrap();

        let args = DoctorArgs {
            fix: false,
            yes: false,
            json: false,
            nupm_home: None,
        };
        let report = run_checks(&args, root).unwrap();
        assert!(report
            .findings
            .iter()
            .any(|f| f.id == "activation.plugin_stale"));
    }
}
