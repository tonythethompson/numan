use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::core::integrity::compute_sha256;
use crate::nupm_compat::classify::{classify_source_root, NupmCompatibility};
use crate::nupm_compat::metadata::read_metadata_limited;
use crate::nupm_compat::schema::{METADATA_FILENAME, NUPM_IMPORT_ORIGIN};
use crate::nupm_compat::walk::check_module_tree_safe;
use crate::state::lockfile::{compute_revision_id, Lockfile};
use crate::state::nupm_import::NupmImportsFile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriftStatus {
    Unchanged,
    SourceMissing,
    MetadataChanged,
    PayloadChanged,
    UnsafeSourceTreeChange,
    CannotCompare { reason: &'static str },
}

impl DriftStatus {
    pub fn is_drift(&self) -> bool {
        !matches!(
            self,
            DriftStatus::Unchanged | DriftStatus::CannotCompare { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriftReport {
    pub package_id: String,
    pub status: DriftStatus,
    pub recorded_source: PathBuf,
    pub installed_revision_id: Option<String>,
    pub recorded_source_payload_sha256: String,
    pub live_source_payload_sha256: Option<String>,
    pub recorded_metadata_sha256: String,
    pub live_metadata_sha256: Option<String>,
}

pub fn compare_import(root: &Path, package_id: &str) -> Result<DriftReport> {
    let lockfile = Lockfile::load(root)?;
    let entry = match lockfile.packages.get(package_id) {
        Some(e) => e,
        None => {
            return Ok(DriftReport {
                package_id: package_id.to_string(),
                status: DriftStatus::CannotCompare {
                    reason: "package not in lockfile",
                },
                recorded_source: PathBuf::new(),
                installed_revision_id: None,
                recorded_source_payload_sha256: String::new(),
                live_source_payload_sha256: None,
                recorded_metadata_sha256: String::new(),
                live_metadata_sha256: None,
            });
        }
    };

    if entry.origin.as_deref() != Some(NUPM_IMPORT_ORIGIN) {
        return Ok(DriftReport {
            package_id: package_id.to_string(),
            status: DriftStatus::CannotCompare {
                reason: "not a nupm import",
            },
            recorded_source: PathBuf::new(),
            installed_revision_id: entry.revision_id.clone(),
            recorded_source_payload_sha256: String::new(),
            live_source_payload_sha256: None,
            recorded_metadata_sha256: String::new(),
            live_metadata_sha256: None,
        });
    }

    let imports = NupmImportsFile::load(root)?;
    let record = match imports.imports.get(package_id) {
        Some(r) => r,
        None => {
            return Ok(DriftReport {
                package_id: package_id.to_string(),
                status: DriftStatus::CannotCompare {
                    reason: "no nupm import provenance",
                },
                recorded_source: PathBuf::new(),
                installed_revision_id: entry.revision_id.clone(),
                recorded_source_payload_sha256: String::new(),
                live_source_payload_sha256: None,
                recorded_metadata_sha256: String::new(),
                live_metadata_sha256: None,
            });
        }
    };

    let recorded_source = PathBuf::from(&record.nupm_source_path);
    if record.nupm_source_path.is_empty() || !recorded_source.exists() {
        return Ok(DriftReport {
            package_id: package_id.to_string(),
            status: DriftStatus::SourceMissing,
            recorded_source,
            installed_revision_id: entry.revision_id.clone(),
            recorded_source_payload_sha256: record.source_payload_sha256.clone(),
            live_source_payload_sha256: None,
            recorded_metadata_sha256: record.nupm_metadata_sha256.clone(),
            live_metadata_sha256: None,
        });
    }

    let (compat, parsed) = classify_source_root(&recorded_source)?;

    if compat != NupmCompatibility::ImportableModule {
        return Ok(DriftReport {
            package_id: package_id.to_string(),
            status: DriftStatus::UnsafeSourceTreeChange,
            recorded_source,
            installed_revision_id: entry.revision_id.clone(),
            recorded_source_payload_sha256: record.source_payload_sha256.clone(),
            live_source_payload_sha256: None,
            recorded_metadata_sha256: record.nupm_metadata_sha256.clone(),
            live_metadata_sha256: None,
        });
    }

    let parsed = parsed.with_context(|| {
        format!(
            "No supported metadata at recorded source '{}'",
            recorded_source.display()
        )
    })?;

    let module_src = recorded_source.join(&parsed.name);
    if check_module_tree_safe(&module_src).is_err() {
        return Ok(DriftReport {
            package_id: package_id.to_string(),
            status: DriftStatus::UnsafeSourceTreeChange,
            recorded_source,
            installed_revision_id: entry.revision_id.clone(),
            recorded_source_payload_sha256: record.source_payload_sha256.clone(),
            live_source_payload_sha256: None,
            recorded_metadata_sha256: record.nupm_metadata_sha256.clone(),
            live_metadata_sha256: None,
        });
    }

    let metadata_path = recorded_source.join(METADATA_FILENAME);
    let metadata_bytes = read_metadata_limited(&metadata_path)?;
    let live_metadata_sha256 = compute_sha256(&metadata_bytes);
    let metadata_changed = live_metadata_sha256 != record.nupm_metadata_sha256;

    let live_source_payload_sha256 = compute_revision_id(&module_src).with_context(|| {
        format!(
            "Failed to hash live source payload at {}",
            module_src.display()
        )
    })?;
    let payload_changed = live_source_payload_sha256 != record.source_payload_sha256;

    let status = if !metadata_changed && !payload_changed {
        DriftStatus::Unchanged
    } else if metadata_changed {
        DriftStatus::MetadataChanged
    } else {
        DriftStatus::PayloadChanged
    };

    Ok(DriftReport {
        package_id: package_id.to_string(),
        status,
        recorded_source,
        installed_revision_id: entry.revision_id.clone(),
        recorded_source_payload_sha256: record.source_payload_sha256.clone(),
        live_source_payload_sha256: Some(live_source_payload_sha256),
        recorded_metadata_sha256: record.nupm_metadata_sha256.clone(),
        live_metadata_sha256: Some(live_metadata_sha256),
    })
}

pub fn count_drifted_imports(root: &Path) -> Result<usize> {
    let lockfile = Lockfile::load(root)?;
    let mut count = 0usize;
    for package_id in lockfile
        .packages
        .iter()
        .filter(|(_, e)| e.origin.as_deref() == Some(NUPM_IMPORT_ORIGIN))
        .map(|(id, _)| id.as_str())
    {
        let report = compare_import(root, package_id)?;
        if report.status.is_drift() {
            count += 1;
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::package::ScopedId;
    use crate::nu::autoload::FakeCandidateRunner;
    use crate::nupm_compat::import::import_module_with_runner;
    use std::fs;

    fn fixture(path: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/nupm")
            .join(path)
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

    #[test]
    fn unchanged_after_import() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("pkg");
        copy_dir_all(&fixture("supported/minimal-module"), &source).unwrap();

        let root = tempfile::tempdir().unwrap();
        import_module_with_runner(
            root.path(),
            &source,
            &ScopedId::parse("test/minimal").unwrap(),
            true,
            &FakeCandidateRunner::success(),
        )
        .unwrap();

        let report = compare_import(root.path(), "test/minimal").unwrap();
        assert_eq!(report.status, DriftStatus::Unchanged);
    }

    #[test]
    fn payload_changed_when_mod_nu_edited() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("pkg");
        copy_dir_all(&fixture("supported/minimal-module"), &source).unwrap();

        let root = tempfile::tempdir().unwrap();
        import_module_with_runner(
            root.path(),
            &source,
            &ScopedId::parse("test/minimal").unwrap(),
            true,
            &FakeCandidateRunner::success(),
        )
        .unwrap();

        fs::write(
            source.join("minimal-module/mod.nu"),
            b"export def changed [] { 99 }",
        )
        .unwrap();

        let report = compare_import(root.path(), "test/minimal").unwrap();
        assert_eq!(report.status, DriftStatus::PayloadChanged);
        assert_ne!(
            report.recorded_source_payload_sha256,
            report.live_source_payload_sha256.unwrap()
        );
    }

    #[test]
    fn metadata_changed_when_nuon_edited() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("pkg");
        copy_dir_all(&fixture("supported/minimal-module"), &source).unwrap();

        let root = tempfile::tempdir().unwrap();
        import_module_with_runner(
            root.path(),
            &source,
            &ScopedId::parse("test/minimal").unwrap(),
            true,
            &FakeCandidateRunner::success(),
        )
        .unwrap();

        let meta_path = source.join("nupm.nuon");
        fs::write(
            &meta_path,
            br#"{
    name: minimal-module
    type: module
    version: "0.1.1"
}"#,
        )
        .unwrap();

        let report = compare_import(root.path(), "test/minimal").unwrap();
        assert_eq!(report.status, DriftStatus::MetadataChanged);
    }

    #[test]
    fn source_missing_when_tree_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("pkg");
        copy_dir_all(&fixture("supported/minimal-module"), &source).unwrap();

        let root = tempfile::tempdir().unwrap();
        import_module_with_runner(
            root.path(),
            &source,
            &ScopedId::parse("test/minimal").unwrap(),
            true,
            &FakeCandidateRunner::success(),
        )
        .unwrap();

        fs::remove_dir_all(&source).unwrap();

        let report = compare_import(root.path(), "test/minimal").unwrap();
        assert_eq!(report.status, DriftStatus::SourceMissing);
    }

    #[test]
    fn cannot_compare_non_nupm_package() {
        let root = tempfile::tempdir().unwrap();
        let report = compare_import(root.path(), "missing/pkg").unwrap();
        assert!(matches!(report.status, DriftStatus::CannotCompare { .. }));
    }

    #[test]
    fn count_drifted_imports_excludes_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("pkg");
        copy_dir_all(&fixture("supported/minimal-module"), &source).unwrap();

        let root = tempfile::tempdir().unwrap();
        import_module_with_runner(
            root.path(),
            &source,
            &ScopedId::parse("test/minimal").unwrap(),
            true,
            &FakeCandidateRunner::success(),
        )
        .unwrap();

        assert_eq!(count_drifted_imports(root.path()).unwrap(), 0);

        fs::write(
            source.join("minimal-module/mod.nu"),
            b"export def drift [] { 1 }",
        )
        .unwrap();
        assert_eq!(count_drifted_imports(root.path()).unwrap(), 1);
    }

    #[test]
    fn unsafe_source_tree_when_metadata_invalid() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("pkg");
        copy_dir_all(&fixture("supported/minimal-module"), &source).unwrap();

        let root = tempfile::tempdir().unwrap();
        import_module_with_runner(
            root.path(),
            &source,
            &ScopedId::parse("test/minimal").unwrap(),
            true,
            &FakeCandidateRunner::success(),
        )
        .unwrap();

        fs::write(source.join("nupm.nuon"), b"not valid nuon {{{").unwrap();

        let report = compare_import(root.path(), "test/minimal").unwrap();
        assert_eq!(report.status, DriftStatus::UnsafeSourceTreeChange);
        assert_eq!(count_drifted_imports(root.path()).unwrap(), 1);
    }
}
