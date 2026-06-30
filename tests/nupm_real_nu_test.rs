//! Real-Nu acceptance tests for Phase 6.4 nupm import.
//!
//! Requires `nu` on `$PATH`. Run with:
//!   cargo test --test nupm_real_nu_test -- --ignored

use std::path::{Path, PathBuf};

use numan_cli::core::package::{ModuleImportMode, ScopedId};
use numan_cli::nu::autoload::{
    generate_autoload_content, resolve_entry, FakeCandidateRunner, ResolvedEntry,
};
use numan_cli::nupm_compat::import_module_with_runner;
use tempfile::TempDir;

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/nupm")
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn which_nu() -> Option<PathBuf> {
    std::env::var("PATH").ok().and_then(|path| {
        path.split(':').map(PathBuf::from).find_map(|dir| {
            let candidate = dir.join("nu");
            if candidate.is_file() {
                Some(candidate)
            } else {
                None
            }
        })
    })
}

#[cfg(windows)]
fn which_nu() -> Option<PathBuf> {
    std::env::var("PATH").ok().and_then(|path| {
        path.split(';').map(PathBuf::from).find_map(|dir| {
            let candidate = dir.join("nu.exe");
            if candidate.is_file() {
                Some(candidate)
            } else {
                None
            }
        })
    })
}

#[cfg(not(any(unix, windows)))]
fn which_nu() -> Option<PathBuf> {
    None
}

#[test]
#[ignore = "requires real Nu binary on $PATH — run in platform acceptance job"]
fn real_nu_imported_module_autoload_validates() {
    let nu = match which_nu() {
        Some(p) => p,
        None => {
            eprintln!("Skipping: Nu binary not found on PATH");
            return;
        }
    };

    let tmp = TempDir::new().unwrap();
    let source = tmp.path().join("pkg");
    copy_dir_all(&fixtures_root().join("supported/minimal-module"), &source).unwrap();

    let root = TempDir::new().unwrap();
    let imported = import_module_with_runner(
        root.path(),
        &source,
        &ScopedId::parse("test/minimal").unwrap(),
        true,
        &FakeCandidateRunner::success(),
    )
    .unwrap();

    let resolved = resolve_entry(
        root.path(),
        &imported.payload_path,
        "mod.nu",
        ModuleImportMode::Module,
        "test/minimal",
    )
    .unwrap();
    let content = generate_autoload_content(&[resolved]).unwrap();
    let candidate = root.path().join(".numan-real-nu.candidate.tmp");
    std::fs::write(&candidate, content.as_bytes()).unwrap();

    let output = std::process::Command::new(&nu)
        .arg("-n")
        .arg(&candidate)
        .output()
        .expect("Failed to spawn Nu");

    assert!(
        output.status.success(),
        "Imported nupm module autoload must pass `nu -n`.\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "requires real Nu binary on $PATH — run in platform acceptance job"]
fn real_nu_unicode_path_import_payload_validates() {
    let nu = match which_nu() {
        Some(p) => p,
        None => {
            eprintln!("Skipping: Nu binary not found on PATH");
            return;
        }
    };

    let tmp = TempDir::new().unwrap();
    let source = tmp.path().join("módulo pkg");
    copy_dir_all(&fixtures_root().join("supported/minimal-module"), &source).unwrap();

    let root = TempDir::new().unwrap();
    let imported = import_module_with_runner(
        root.path(),
        &source,
        &ScopedId::parse("test/unicode").unwrap(),
        true,
        &FakeCandidateRunner::success(),
    )
    .unwrap();

    let payload_mod = root.path().join(&imported.payload_path).join("mod.nu");
    assert!(payload_mod.is_file());

    let output = std::process::Command::new(&nu)
        .arg("-n")
        .arg(&payload_mod)
        .output()
        .expect("Failed to spawn Nu");

    assert!(
        output.status.success(),
        "Unicode-path imported mod.nu must pass `nu -n`.\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "requires real Nu binary on $PATH — run in platform acceptance job"]
fn real_nu_rendered_use_with_spaces_validates() {
    let nu = match which_nu() {
        Some(p) => p,
        None => {
            eprintln!("Skipping: Nu binary not found on PATH");
            return;
        }
    };

    let dir = TempDir::new().unwrap();
    let module_file = dir.path().join("my module.nu");
    std::fs::write(&module_file, b"export def hello [] { \"hello\" }\n").unwrap();

    let entry = ResolvedEntry {
        absolute_path: module_file.clone(),
        import_mode: ModuleImportMode::Module,
        scoped_id: "owner/my-mod".to_string(),
    };
    let content = generate_autoload_content(&[entry]).unwrap();
    let candidate = dir.path().join(".candidate.tmp");
    std::fs::write(&candidate, content.as_bytes()).unwrap();

    let output = std::process::Command::new(&nu)
        .arg("-n")
        .arg(&candidate)
        .output()
        .expect("Failed to spawn Nu");

    assert!(
        output.status.success(),
        "Autoload with spaced path must pass `nu -n`.\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
