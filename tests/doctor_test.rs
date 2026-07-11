//! Integration tests for `numan doctor`.

use numan_cli::cmd::doctor::{
    execute_with_options, run_checks_with_options, DoctorArgs, DoctorOptions, Severity,
};
use numan_cli::cmd::init::{execute_with_runner, InitArgs};
use numan_cli::core::integrity;
use numan_cli::nu::autoload::FakeCandidateRunner;
use numan_cli::nu::bootstrap::managed_nu_binary;
use numan_cli::nu::paths::NuPaths;
use numan_cli::state::journal::{PendingActivation, PendingActivationEntry, PendingStatus};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tempfile::TempDir;

static TEST_OFF_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);
static TEST_NU_SETUP_CALLED: Mutex<bool> = Mutex::new(false);

fn discover_off_path_test() -> Option<PathBuf> {
    TEST_OFF_PATH.lock().ok()?.clone()
}

fn nu_setup_repair_test(
    args: &numan_cli::cmd::setup::NuSetupArgs,
    _root: &Path,
) -> anyhow::Result<()> {
    let expected = TEST_OFF_PATH.lock().unwrap();
    assert_eq!(
        args.use_existing.as_deref(),
        expected.as_ref().map(|p| p.as_path())
    );
    assert!(args.yes);
    *TEST_NU_SETUP_CALLED.lock().unwrap() = true;
    Ok(())
}

struct ClearedPath {
    saved: Option<String>,
}

impl ClearedPath {
    fn new() -> Self {
        let saved = std::env::var("PATH").ok();
        std::env::set_var("PATH", "");
        Self { saved }
    }
}

impl Drop for ClearedPath {
    fn drop(&mut self) {
        match &self.saved {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }
    }
}

fn fake_runner(_exe: &str) -> Box<dyn numan_cli::nu::autoload::CandidateRunner> {
    Box::new(FakeCandidateRunner::success())
}

fn fake_init(args: &InitArgs, root: &Path) -> anyhow::Result<()> {
    let nu_exe = managed_nu_binary(root);
    std::fs::create_dir_all(nu_exe.parent().unwrap()).unwrap();
    std::fs::write(&nu_exe, b"nu").unwrap();
    let bytes = std::fs::read(&nu_exe).unwrap();
    let paths = NuPaths {
        nu_executable: nu_exe.to_string_lossy().into_owned(),
        nu_version: "0.113.1".to_string(),
        plugin_registry_path: root.join("plugins.msgpackz").to_string_lossy().into_owned(),
        nu_executable_hash: integrity::compute_sha256(&bytes),
        platform: "test".to_string(),
        data_dir: None,
        vendor_autoload_dirs: vec![],
        vendor_autoload_dir: None,
    };
    execute_with_runner(args, root, move || Ok(paths.clone()), fake_runner)
}

#[test]
fn doctor_report_only_leaves_root_unchanged() {
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

#[test]
fn doctor_fix_auto_creates_layout_without_network() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root).unwrap();
    std::env::set_var("NUMAN_ROOT", root);
    let nu_exe = managed_nu_binary(root);
    std::fs::create_dir_all(nu_exe.parent().unwrap()).unwrap();
    std::fs::write(&nu_exe, b"nu").unwrap();

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
            init_repair: Some(fake_init),
            activate_repair: None,
            nu_setup_repair: None,
            discover_off_path: None,
        },
    )
    .unwrap();
    assert_eq!(code, 0);
    assert!(root.join("state").is_dir());
    assert!(root.join("nu_state/paths.json").is_file());
}

#[test]
fn doctor_reports_pending_plugin_journal() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    fake_init(&InitArgs { refresh: false }, root).unwrap();

    let paths = NuPaths::load(root).unwrap();
    let journal = PendingActivation {
        nu_executable_sha256: paths.nu_executable_hash.clone(),
        nu_version: paths.nu_version.clone(),
        plugin_registry_path: paths.plugin_registry_path.clone(),
        created_at: "now".to_string(),
        entries: vec![PendingActivationEntry {
            package_id: "owner/plugin".to_string(),
            payload_path: "packages/plugins/owner/plugin/1.0.0-abc".to_string(),
            executable_path: "nu_plugin_test".to_string(),
            absolute_binary_path: root
                .join("packages/plugins/owner/plugin/1.0.0-abc/nu_plugin_test")
                .to_string_lossy()
                .into_owned(),
            status: PendingStatus::Prepared,
            error: None,
        }],
    };
    journal.save(root).unwrap();

    let args = DoctorArgs {
        fix: false,
        yes: false,
        json: false,
        nupm_home: None,
    };
    let report = run_checks_with_options(&args, root, &DoctorOptions::default()).unwrap();
    assert!(report
        .findings
        .iter()
        .any(|f| f.id == "journal.plugin_pending"));
}

#[test]
fn doctor_detects_nu_path_drift() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let nu_exe = root.join("nu");
    std::fs::write(&nu_exe, b"v1").unwrap();
    let paths = NuPaths {
        nu_executable: nu_exe.to_string_lossy().into_owned(),
        nu_version: "0.113.1".to_string(),
        plugin_registry_path: root.join("plugins.msgpackz").to_string_lossy().into_owned(),
        nu_executable_hash: integrity::compute_sha256(b"stale"),
        platform: "test".to_string(),
        data_dir: None,
        vendor_autoload_dirs: vec![],
        vendor_autoload_dir: None,
    };
    paths.save(root).unwrap();

    let args = DoctorArgs {
        fix: false,
        yes: false,
        json: false,
        nupm_home: None,
    };
    let report = run_checks_with_options(&args, root, &DoctorOptions::default()).unwrap();
    assert!(report
        .findings
        .iter()
        .any(|f| f.id == "nu_paths.drift" && f.severity == Severity::Error));
}

#[test]
fn doctor_reports_off_path_nu_without_download() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root).unwrap();

    let off_path = root.join("off-path-nu.exe");
    std::fs::write(&off_path, b"fake nu").unwrap();

    *TEST_OFF_PATH.lock().unwrap() = Some(off_path.clone());

    let _cleared_path = ClearedPath::new();
    let args = DoctorArgs {
        fix: false,
        yes: false,
        json: false,
        nupm_home: None,
    };
    let report = run_checks_with_options(
        &args,
        root,
        &DoctorOptions {
            discover_off_path: Some(discover_off_path_test),
            ..DoctorOptions::default()
        },
    )
    .unwrap();

    let finding = report
        .findings
        .iter()
        .find(|f| f.id == "nu.binary.found_off_path")
        .expect("expected nu.binary.found_off_path");
    assert_eq!(finding.severity, Severity::Warn);
    assert!(finding.fix.as_deref().unwrap().contains("--use-existing"));

    let missing = report
        .findings
        .iter()
        .find(|f| f.id == "nu.binary.missing_on_path")
        .expect("expected nu.binary.missing_on_path");
    assert_eq!(missing.severity, Severity::Ok);
}

#[test]
fn doctor_fix_registers_off_path_nu_without_network() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root).unwrap();

    let off_path = root.join("off-path-nu.exe");
    std::fs::write(&off_path, b"fake nu").unwrap();

    *TEST_OFF_PATH.lock().unwrap() = Some(off_path.clone());
    *TEST_NU_SETUP_CALLED.lock().unwrap() = false;

    let _cleared_path = ClearedPath::new();
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
            nu_setup_repair: Some(nu_setup_repair_test),
            discover_off_path: Some(discover_off_path_test),
            ..DoctorOptions::default()
        },
    )
    .unwrap();

    assert!(*TEST_NU_SETUP_CALLED.lock().unwrap());
    assert_eq!(code, 1);
}
