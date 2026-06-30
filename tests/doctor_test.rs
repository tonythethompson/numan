//! Integration tests for `numan doctor`.

use numan_cli::cmd::doctor::{
    execute_with_options, run_checks, DoctorArgs, DoctorOptions, Severity,
};
use numan_cli::cmd::init::{execute_with_runner, InitArgs};
use numan_cli::core::integrity;
use numan_cli::nu::autoload::FakeCandidateRunner;
use numan_cli::nu::paths::NuPaths;
use numan_cli::state::journal::{PendingActivation, PendingActivationEntry, PendingStatus};
use std::path::Path;
use tempfile::TempDir;

fn fake_runner(_exe: &str) -> Box<dyn numan_cli::nu::autoload::CandidateRunner> {
    Box::new(FakeCandidateRunner::success())
}

fn fake_init(args: &InitArgs, root: &Path) -> anyhow::Result<()> {
    let nu_exe = root.join("nu");
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
    let report = run_checks(&args, root).unwrap();
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
    let report = run_checks(&args, root).unwrap();
    assert!(report
        .findings
        .iter()
        .any(|f| f.id == "nu_paths.drift" && f.severity == Severity::Error));
}
