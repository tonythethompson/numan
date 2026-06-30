//! Phase 6.1 integration tests (T13–T15).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use numan_cli::cmd::nupm::{self, NupmArgs, NupmCommands, StatusArgs};
use numan_cli::nupm_compat::schema::METADATA_FILENAME;
use numan_cli::util::fs_safety::is_symlink_or_reparse;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/nupm")
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dest)?;
        } else {
            fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ManifestEntry {
    rel_path: String,
    is_symlink: bool,
    is_dir: bool,
    sha256: Option<String>,
}

fn fixture_manifest(root: &Path) -> BTreeMap<String, ManifestEntry> {
    let mut out = BTreeMap::new();
    walk_manifest(root, root, &mut out);
    out
}

fn walk_manifest(base: &Path, dir: &Path, out: &mut BTreeMap<String, ManifestEntry>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let rel = path
            .strip_prefix(base)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let meta = fs::symlink_metadata(&path).ok();
        let is_symlink = is_symlink_or_reparse(&path).unwrap_or(false);
        let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        let sha256 = if meta.as_ref().is_some_and(|m| m.is_file()) && !is_symlink {
            let bytes = fs::read(&path).unwrap_or_default();
            Some(hex::encode(Sha256::digest(bytes)))
        } else {
            None
        };
        out.insert(
            rel.clone(),
            ManifestEntry {
                rel_path: rel,
                is_symlink,
                is_dir,
                sha256,
            },
        );
        if is_dir && !is_symlink {
            walk_manifest(base, &path, out);
        }
    }
}

#[test]
fn t13_nupm_home_layout_installed_only() {
    let home = fixtures_root().join("nupm-home-layout");
    let scan = numan_cli::nupm_compat::scan_nupm_home(&home).unwrap();
    assert_eq!(scan.installed_only.len(), 1);
    assert!(scan
        .source_roots
        .iter()
        .all(|r| r.compatibility != numan_cli::nupm_compat::NupmCompatibility::ImportableModule));
}

#[test]
fn t14_inspect_all_without_home_errors_status_ok() {
    let prev = std::env::var_os("NUPM_HOME");
    std::env::remove_var("NUPM_HOME");

    let root = TempDir::new().unwrap();
    let mut buf = Vec::new();
    let status_args = NupmArgs {
        command: NupmCommands::Status(StatusArgs { nupm_home: None }),
    };
    nupm::execute(&status_args, root.path(), &mut buf).unwrap();
    assert!(String::from_utf8(buf).unwrap().contains("not configured"));

    let inspect_args = NupmArgs {
        command: NupmCommands::Inspect(nupm::InspectArgs {
            all: true,
            path: None,
            nupm_home: None,
        }),
    };
    let mut buf2 = Vec::new();
    assert!(nupm::execute(&inspect_args, root.path(), &mut buf2).is_err());

    if let Some(p) = prev {
        std::env::set_var("NUPM_HOME", p);
    }
}

#[test]
fn status_fails_on_corrupt_lockfile() {
    let root = TempDir::new().unwrap();
    std::fs::write(root.path().join("lockfile"), b"{not json").unwrap();
    let args = NupmArgs {
        command: NupmCommands::Status(StatusArgs { nupm_home: None }),
    };
    let mut buf = Vec::new();
    assert!(nupm::execute(&args, root.path(), &mut buf).is_err());
}

#[test]
fn t15_no_mutation_under_nupm_home_fixture() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("nupm-home");
    copy_dir_all(&fixtures_root().join("nupm-home-layout"), &home).unwrap();

    let before = fixture_manifest(&home);

    let root = TempDir::new().unwrap();
    let mut buf = Vec::new();
    let args = NupmArgs {
        command: NupmCommands::Status(StatusArgs {
            nupm_home: Some(home.clone()),
        }),
    };
    nupm::execute(&args, root.path(), &mut buf).unwrap();

    let mut buf2 = Vec::new();
    let inspect_args = NupmArgs {
        command: NupmCommands::Inspect(nupm::InspectArgs {
            all: true,
            path: None,
            nupm_home: Some(home.clone()),
        }),
    };
    nupm::execute(&inspect_args, root.path(), &mut buf2).unwrap();

    let path = fixtures_root().join("supported/minimal-module");
    let mut buf3 = Vec::new();
    let single = NupmArgs {
        command: NupmCommands::Inspect(nupm::InspectArgs {
            all: false,
            path: Some(path),
            nupm_home: None,
        }),
    };
    nupm::execute(&single, root.path(), &mut buf3).unwrap();

    let after = fixture_manifest(&home);
    assert_eq!(before, after);
}

#[test]
fn inspect_supported_minimal_module() {
    let root = TempDir::new().unwrap();
    let path = fixtures_root().join("supported/minimal-module");
    let mut buf = Vec::new();
    let args = NupmArgs {
        command: NupmCommands::Inspect(nupm::InspectArgs {
            all: false,
            path: Some(path),
            nupm_home: None,
        }),
    };
    nupm::execute(&args, root.path(), &mut buf).unwrap();
    let out = String::from_utf8(buf).unwrap();
    assert!(out.contains("ImportableModule"));
    assert!(out.contains("Eligible:     yes"));
}

// silence unused import warning for METADATA_FILENAME if not used
#[allow(dead_code)]
fn _metadata_name() -> &'static str {
    METADATA_FILENAME
}
