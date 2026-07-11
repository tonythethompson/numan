//! `numan setup nu` integration tests.

use numan_cli::cmd::setup::{execute_nu, NuSetupArgs};
use numan_cli::core::platform::Platform;
use numan_cli::nu::bootstrap::{self, install_from_archive, NuSetupOptions};
use numan_cli::nu::paths::find_nu_executable_with_root;
use std::io::Write;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

#[test]
fn managed_nu_is_discovered_after_install() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::env::set_var("NUMAN_ROOT", root);
    let zip_path = root.join("nu-test.zip");

    {
        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        let inner = if cfg!(windows) {
            "nu-0.0.0-test/nu.exe"
        } else {
            "nu-0.0.0-test/nu"
        };
        zip.start_file(inner, options).unwrap();
        zip.write_all(b"fake nu binary").unwrap();
        zip.finish().unwrap();
    }

    install_from_archive(&zip_path, root, "0.0.0-test").unwrap();
    bootstrap::prepend_process_path(&bootstrap::managed_nu_dir(root)).unwrap();

    let resolved = find_nu_executable_with_root(root).unwrap();
    let expected = bootstrap::managed_nu_binary(root);
    assert_eq!(
        std::fs::canonicalize(&resolved).unwrap(),
        std::fs::canonicalize(&expected).unwrap(),
    );
}

#[test]
fn setup_nu_uses_injected_installer_without_network() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let platform = Platform::detect();

    let installer = |install_root: &std::path::Path, _platform: &Platform| {
        let binary = bootstrap::managed_nu_binary(install_root);
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, b"fake nu").unwrap();
        Ok(binary)
    };

    bootstrap::execute_nu_setup_with_installer(
        root,
        &platform,
        &NuSetupOptions {
            yes: true,
            force: false,
            skip_path: true,
        },
        installer,
    )
    .unwrap();

    assert!(bootstrap::managed_nu_binary(root).is_file());
}

#[test]
fn execute_nu_command_wraps_installer() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    // Pre-install managed binary so execute_nu short-circuits without network.
    let binary = bootstrap::managed_nu_binary(root);
    std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
    std::fs::write(&binary, b"fake nu").unwrap();

    execute_nu(
        &NuSetupArgs {
            force: false,
            skip_path: true,
            yes: true,
            use_existing: None,
        },
        root,
    )
    .unwrap();
}

#[test]
fn setup_nu_use_existing_registers_binary_without_download() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let existing_dir = dir.path().join("existing-nu");
    std::fs::create_dir_all(&existing_dir).unwrap();
    let existing = existing_dir.join(if cfg!(windows) { "nu.exe" } else { "nu" });
    std::fs::write(&existing, b"fake nu").unwrap();

    execute_nu(
        &NuSetupArgs {
            force: false,
            skip_path: true,
            yes: true,
            use_existing: Some(existing.clone()),
        },
        root,
    )
    .unwrap();

    assert!(
        !bootstrap::managed_nu_binary(root).is_file(),
        "use-existing should not install a managed copy under NUMAN_ROOT"
    );

    let path_var = std::env::var("PATH").unwrap();
    let parent = existing
        .canonicalize()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let parent_str = parent.to_string_lossy().replace("\\\\?\\", "");
    assert!(
        path_var
            .split(';')
            .any(|part| part.trim().eq_ignore_ascii_case(&parent_str)),
        "PATH should contain the existing Nu directory after use-existing"
    );
}
