use std::path::{Path, PathBuf};

use anyhow::Result;

use super::metadata::{parse_metadata, read_metadata_limited, MetadataError, ParsedMetadata};
use super::schema::{BUILD_SCRIPT_NAME, METADATA_FILENAME, MODULE_ENTRY};
use super::walk::{
    check_path_chain_safe, check_path_chain_safe_within, find_package_root, is_safe_package_name,
};
use crate::util::fs_safety::is_symlink_or_reparse;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NupmCompatibility {
    ImportableModule,
    DeferredScript,
    UnsupportedCustomBuild,
    UnsupportedDependencies,
    InvalidMetadata,
    UnsafeFilesystemLayout,
    UnknownType,
}

pub struct ClassifyContext {
    pub package_root: PathBuf,
    pub metadata_path: PathBuf,
    pub has_build_script: bool,
}

pub fn classify_source_root(
    package_root: &Path,
) -> Result<(NupmCompatibility, Option<ParsedMetadata>)> {
    check_path_chain_safe(package_root)?;

    let metadata_path = package_root.join(METADATA_FILENAME);
    if is_symlink_or_reparse(&metadata_path)? {
        return Ok((NupmCompatibility::UnsafeFilesystemLayout, None));
    }
    if !metadata_path.is_file() {
        return Ok((NupmCompatibility::InvalidMetadata, None));
    }

    let bytes = match read_metadata_limited(&metadata_path) {
        Ok(b) => b,
        Err(MetadataError::InputTooLarge) => {
            return Ok((NupmCompatibility::InvalidMetadata, None));
        }
        Err(MetadataError::Io(_)) => {
            return Ok((NupmCompatibility::UnsafeFilesystemLayout, None));
        }
        Err(_) => return Ok((NupmCompatibility::InvalidMetadata, None)),
    };

    let parsed = match parse_metadata(&bytes) {
        Ok(p) => p,
        Err(_) => return Ok((NupmCompatibility::InvalidMetadata, None)),
    };

    let ctx = ClassifyContext {
        package_root: package_root.to_path_buf(),
        metadata_path,
        has_build_script: package_root.join(BUILD_SCRIPT_NAME).is_file(),
    };

    Ok((classify_parsed(&ctx, &parsed), Some(parsed)))
}

pub fn classify_parsed(ctx: &ClassifyContext, parsed: &ParsedMetadata) -> NupmCompatibility {
    // Step 3 — layout checks (module packages only)
    if parsed.package_type == "module" {
        if !is_safe_package_name(&parsed.name) {
            return NupmCompatibility::UnsafeFilesystemLayout;
        }

        let module_dir = ctx.package_root.join(&parsed.name);
        let entry = module_dir.join(MODULE_ENTRY);

        if !module_dir.is_dir() {
            return NupmCompatibility::UnsafeFilesystemLayout;
        }
        if !entry.is_file() {
            return NupmCompatibility::UnsafeFilesystemLayout;
        }
        if check_path_chain_safe_within(&ctx.package_root, &module_dir).is_err()
            || check_path_chain_safe_within(&ctx.package_root, &entry).is_err()
        {
            return NupmCompatibility::UnsafeFilesystemLayout;
        }
    } else if !is_safe_package_name(&parsed.name) {
        return NupmCompatibility::UnsafeFilesystemLayout;
    }

    // Step 4 — precedence
    if ctx.has_build_script || parsed.package_type == "custom" {
        return NupmCompatibility::UnsupportedCustomBuild;
    }
    if parsed.behavior.has_dependencies {
        return NupmCompatibility::UnsupportedDependencies;
    }
    if parsed.package_type == "script" || parsed.behavior.has_scripts {
        return NupmCompatibility::DeferredScript;
    }
    if !matches!(parsed.package_type.as_str(), "module" | "script" | "custom") {
        return NupmCompatibility::UnknownType;
    }

    NupmCompatibility::ImportableModule
}

pub fn find_source_root(start: &Path) -> Result<Option<PathBuf>> {
    find_package_root(start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn pkg(path: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/nupm")
            .join(path)
    }

    #[test]
    fn t06_minimal_importable() {
        let root = pkg("supported/minimal-module");
        let (c, _) = classify_source_root(&root).unwrap();
        assert_eq!(c, NupmCompatibility::ImportableModule);
    }

    #[test]
    fn t07_script_deferred() {
        let (c, _) = classify_source_root(&pkg("rejected/script-type")).unwrap();
        assert_eq!(c, NupmCompatibility::DeferredScript);
    }

    #[test]
    fn t08_custom_build() {
        let (c, _) = classify_source_root(&pkg("rejected/custom-with-build")).unwrap();
        assert_eq!(c, NupmCompatibility::UnsupportedCustomBuild);
    }

    #[test]
    fn t09_module_scripts() {
        let (c, _) = classify_source_root(&pkg("rejected/module-with-scripts")).unwrap();
        assert_eq!(c, NupmCompatibility::DeferredScript);
    }

    #[test]
    fn t10_external_deps() {
        let (c, _) = classify_source_root(&pkg("rejected/external-deps")).unwrap();
        assert_eq!(c, NupmCompatibility::UnsupportedDependencies);
    }

    #[test]
    fn t11_missing_mod_nu() {
        let (c, _) = classify_source_root(&pkg("rejected/missing-mod-nu")).unwrap();
        assert_eq!(c, NupmCompatibility::UnsafeFilesystemLayout);
    }

    #[test]
    fn t12_unknown_type() {
        let (c, _) = classify_source_root(&pkg("rejected/unknown-type")).unwrap();
        assert_eq!(c, NupmCompatibility::UnknownType);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_metadata_is_unsafe() {
        use std::fs;
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.nuon");
        fs::write(&real, br#"{ name: m, version: "0.1.0", type: module }"#).unwrap();
        let root = dir.path().join("pkg");
        fs::create_dir_all(root.join("m")).unwrap();
        fs::write(root.join("m/mod.nu"), b"").unwrap();
        std::os::unix::fs::symlink(&real, root.join("nupm.nuon")).unwrap();
        let (compat, _) = classify_source_root(&root).unwrap();
        assert_eq!(compat, NupmCompatibility::UnsafeFilesystemLayout);
    }
}
