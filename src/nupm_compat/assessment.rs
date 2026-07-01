use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::classify::{classify_source_root, NupmCompatibility};
use super::metadata::{parse_metadata, MetadataError, ParsedMetadata};
use super::schema::{BUILD_SCRIPT_NAME, METADATA_FILENAME, MODULE_ENTRY};
use super::walk::{
    check_module_tree_safe, check_path_chain_safe, check_path_chain_safe_within,
    is_safe_package_name,
};
use crate::util::fs_safety::is_symlink_or_reparse;

/// Stable, machine-readable outcome class for a nupm migration candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NupmOutcome {
    /// Safe to import under the documented supported rule.
    ImportableNow,
    /// Read-only inspection is supported; migration is deferred pending Numan support.
    InspectOnly,
    /// Requires user-driven migration (dependencies, overlays, build scripts, etc.).
    ManualMigrationRequired,
    /// Cannot be handled by Numan in any form.
    Unsupported,
}

impl NupmOutcome {
    pub fn is_importable(self) -> bool {
        self == NupmOutcome::ImportableNow
    }
}

/// Structured reason codes explaining why a package is not importable_now.
///
/// Consumers must tolerate unknown reason codes for forward compatibility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NupmReasonCode {
    /// No reason; package is importable.
    None,
    /// build.nu is present in the package root.
    CustomBuildNu,
    /// Declared type is script.
    ScriptPackage,
    /// Module metadata declares auxiliary scripts.
    AuxiliaryScripts,
    /// Metadata declares external dependencies.
    DeclaredDependencies,
    /// Package type is not module/script/custom.
    UnknownPackageType,
    /// Metadata shape is outside the supported NUON subset.
    UnsupportedMetadataShape,
    /// Required metadata keys are missing.
    MissingRequiredKeys,
    /// Metadata uses unsupported NUON constructs (closures, variables, etc.).
    UnsupportedNuonConstruct,
    /// Metadata size/depth/field limits exceeded.
    MetadataLimitExceeded,
    /// Filesystem layout is unsafe (symlinks, traversal, special files).
    UnsafeFilesystemLayout,
    /// Module directory matching the declared name is missing.
    MissingModuleDirectory,
    /// Module entry mod.nu is missing.
    MissingModuleEntry,
    /// Metadata file is unavailable for an installed-only directory.
    MetadataUnavailable,
}

impl std::fmt::Display for NupmReasonCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            NupmReasonCode::None => "none",
            NupmReasonCode::CustomBuildNu => "custom_build_nu",
            NupmReasonCode::ScriptPackage => "script_package",
            NupmReasonCode::AuxiliaryScripts => "auxiliary_scripts",
            NupmReasonCode::DeclaredDependencies => "declared_dependencies",
            NupmReasonCode::UnknownPackageType => "unknown_package_type",
            NupmReasonCode::UnsupportedMetadataShape => "unsupported_metadata_shape",
            NupmReasonCode::MissingRequiredKeys => "missing_required_keys",
            NupmReasonCode::UnsupportedNuonConstruct => "unsupported_nuon_construct",
            NupmReasonCode::MetadataLimitExceeded => "metadata_limit_exceeded",
            NupmReasonCode::UnsafeFilesystemLayout => "unsafe_filesystem_layout",
            NupmReasonCode::MissingModuleDirectory => "missing_module_directory",
            NupmReasonCode::MissingModuleEntry => "missing_module_entry",
            NupmReasonCode::MetadataUnavailable => "metadata_unavailable",
        };
        write!(f, "{s}")
    }
}

/// Typed recommended action for a migration candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NupmRecommendedAction {
    /// Import the module with `numan nupm import`.
    Import,
    /// Inspect the source manually before deciding.
    Inspect,
    /// Perform a manual migration (resolve dependencies, run build, etc.).
    ManualMigration,
    /// Repair the source package before retrying.
    RepairSource,
}

/// Detected features of a nupm package that influence migration planning.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedFeatures {
    pub has_scripts: bool,
    pub has_dependencies: bool,
    pub has_build_script: bool,
    pub is_overlay: bool,
}

/// Canonical assessment for a nupm package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NupmAssessment {
    pub compatibility: NupmCompatibility,
    pub outcome: NupmOutcome,
    pub reason_codes: Vec<NupmReasonCode>,
    pub recommended_action: NupmRecommendedAction,
    pub detected_features: DetectedFeatures,
}

impl NupmAssessment {
    pub fn is_importable(&self) -> bool {
        self.outcome == NupmOutcome::ImportableNow
    }
}

/// Build a canonical assessment from the source root.
///
/// This is the canonical path for reporting and import decisions. It uses the
/// existing classifier for the base compatibility and then enriches it with
/// parsed metadata and observed filesystem facts to produce specific reason
/// codes and a recommended action.
pub fn assess_source_root(package_root: &Path) -> Result<(NupmAssessment, Option<ParsedMetadata>)> {
    let (compatibility, metadata) = classify_source_root(package_root)?;
    let assessment = build_assessment(package_root, compatibility, metadata.as_ref());
    Ok((assessment, metadata))
}

fn build_assessment(
    package_root: &Path,
    compatibility: NupmCompatibility,
    metadata: Option<&ParsedMetadata>,
) -> NupmAssessment {
    match compatibility {
        NupmCompatibility::ImportableModule => NupmAssessment {
            compatibility,
            outcome: NupmOutcome::ImportableNow,
            reason_codes: vec![NupmReasonCode::None],
            recommended_action: NupmRecommendedAction::Import,
            detected_features: features_from_metadata(metadata),
        },
        NupmCompatibility::DeferredScript => {
            let mut reason_codes = Vec::new();
            if let Some(parsed) = metadata {
                if parsed.package_type == "script" {
                    reason_codes.push(NupmReasonCode::ScriptPackage);
                }
                if parsed.behavior.has_scripts {
                    reason_codes.push(NupmReasonCode::AuxiliaryScripts);
                }
            }
            if reason_codes.is_empty() {
                reason_codes.push(NupmReasonCode::ScriptPackage);
            }
            NupmAssessment {
                compatibility,
                outcome: NupmOutcome::ManualMigrationRequired,
                reason_codes,
                recommended_action: NupmRecommendedAction::ManualMigration,
                detected_features: features_from_metadata(metadata),
            }
        }
        NupmCompatibility::UnsupportedDependencies => NupmAssessment {
            compatibility,
            outcome: NupmOutcome::ManualMigrationRequired,
            reason_codes: vec![NupmReasonCode::DeclaredDependencies],
            recommended_action: NupmRecommendedAction::ManualMigration,
            detected_features: features_from_metadata(metadata),
        },
        NupmCompatibility::UnsupportedCustomBuild => NupmAssessment {
            compatibility,
            outcome: NupmOutcome::ManualMigrationRequired,
            reason_codes: vec![NupmReasonCode::CustomBuildNu],
            recommended_action: NupmRecommendedAction::ManualMigration,
            detected_features: {
                let mut f = features_from_metadata(metadata);
                f.has_build_script = has_build_script(package_root);
                f
            },
        },
        NupmCompatibility::UnknownType => NupmAssessment {
            compatibility,
            outcome: NupmOutcome::Unsupported,
            reason_codes: vec![NupmReasonCode::UnknownPackageType],
            recommended_action: NupmRecommendedAction::RepairSource,
            detected_features: features_from_metadata(metadata),
        },
        NupmCompatibility::InvalidMetadata => NupmAssessment {
            compatibility,
            outcome: NupmOutcome::Unsupported,
            reason_codes: reason_codes_for_invalid_metadata(package_root),
            recommended_action: NupmRecommendedAction::RepairSource,
            detected_features: DetectedFeatures::default(),
        },
        NupmCompatibility::UnsafeFilesystemLayout => NupmAssessment {
            compatibility,
            outcome: NupmOutcome::Unsupported,
            reason_codes: reason_codes_for_unsafe_layout(package_root, metadata),
            recommended_action: NupmRecommendedAction::RepairSource,
            detected_features: features_from_metadata(metadata),
        },
    }
}

fn features_from_metadata(metadata: Option<&ParsedMetadata>) -> DetectedFeatures {
    let mut f = DetectedFeatures::default();
    if let Some(parsed) = metadata {
        f.has_scripts = parsed.behavior.has_scripts;
        f.has_dependencies = parsed.behavior.has_dependencies;
        f.is_overlay = !matches!(parsed.package_type.as_str(), "module" | "script" | "custom");
    }
    f
}

fn has_build_script(package_root: &Path) -> bool {
    let build_path = package_root.join(BUILD_SCRIPT_NAME);
    match std::fs::symlink_metadata(&build_path) {
        Ok(_) => !is_symlink_or_reparse(&build_path).unwrap_or(true),
        Err(_) => false,
    }
}

fn reason_codes_for_invalid_metadata(package_root: &Path) -> Vec<NupmReasonCode> {
    let metadata_path = package_root.join(METADATA_FILENAME);
    if !metadata_path.is_file() {
        return vec![NupmReasonCode::MissingRequiredKeys];
    }
    let bytes = match super::metadata::read_metadata_limited(&metadata_path) {
        Ok(b) => b,
        Err(MetadataError::InputTooLarge) => return vec![NupmReasonCode::MetadataLimitExceeded],
        Err(_) => return vec![NupmReasonCode::UnsupportedMetadataShape],
    };
    match parse_metadata(&bytes) {
        Ok(_) => vec![NupmReasonCode::UnsupportedMetadataShape],
        Err(MetadataError::InputTooLarge) => vec![NupmReasonCode::MetadataLimitExceeded],
        Err(MetadataError::InvalidSyntax(msg)) => {
            if msg.contains("required") {
                vec![NupmReasonCode::MissingRequiredKeys]
            } else {
                vec![NupmReasonCode::UnsupportedNuonConstruct]
            }
        }
        Err(MetadataError::MissingRequiredField(_)) => vec![NupmReasonCode::MissingRequiredKeys],
        Err(MetadataError::DuplicateField(_)) | Err(MetadataError::UnknownField(_)) => {
            vec![NupmReasonCode::UnsupportedMetadataShape]
        }
        Err(MetadataError::LimitExceeded(_)) => vec![NupmReasonCode::MetadataLimitExceeded],
        Err(MetadataError::Io(_)) => vec![NupmReasonCode::UnsupportedMetadataShape],
    }
}

fn reason_codes_for_unsafe_layout(
    package_root: &Path,
    metadata: Option<&ParsedMetadata>,
) -> Vec<NupmReasonCode> {
    let metadata_path = package_root.join(METADATA_FILENAME);
    if metadata_path.exists() && is_symlink_or_reparse(&metadata_path).unwrap_or(true) {
        return vec![NupmReasonCode::UnsafeFilesystemLayout];
    }
    if check_path_chain_safe(package_root).is_err() {
        return vec![NupmReasonCode::UnsafeFilesystemLayout];
    }
    if let Some(parsed) = metadata {
        if !is_safe_package_name(&parsed.name) {
            return vec![NupmReasonCode::UnsafeFilesystemLayout];
        }
        if parsed.package_type == "module" {
            let module_dir = package_root.join(&parsed.name);
            if !module_dir.is_dir() {
                return vec![NupmReasonCode::MissingModuleDirectory];
            }
            let entry = module_dir.join(MODULE_ENTRY);
            if !entry.is_file() {
                return vec![NupmReasonCode::MissingModuleEntry];
            }
            if check_path_chain_safe_within(package_root, &module_dir).is_err()
                || check_path_chain_safe_within(package_root, &entry).is_err()
                || check_module_tree_safe(&module_dir).is_err()
            {
                return vec![NupmReasonCode::UnsafeFilesystemLayout];
            }
        }
    }
    vec![NupmReasonCode::UnsafeFilesystemLayout]
}

/// Build an assessment for an installed-only directory with no metadata.
pub fn installed_only_assessment(_name: &str) -> NupmAssessment {
    NupmAssessment {
        compatibility: NupmCompatibility::InvalidMetadata,
        outcome: NupmOutcome::InspectOnly,
        reason_codes: vec![NupmReasonCode::MetadataUnavailable],
        recommended_action: NupmRecommendedAction::Inspect,
        detected_features: DetectedFeatures::default(),
    }
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
        let (a, _) = assess_source_root(&pkg("supported/minimal-module")).unwrap();
        assert_eq!(a.outcome, NupmOutcome::ImportableNow);
        assert_eq!(a.reason_codes, vec![NupmReasonCode::None]);
        assert_eq!(a.recommended_action, NupmRecommendedAction::Import);
    }

    #[test]
    fn t07_script_deferred() {
        let (a, _) = assess_source_root(&pkg("rejected/script-type")).unwrap();
        assert_eq!(a.outcome, NupmOutcome::ManualMigrationRequired);
        assert!(a.reason_codes.contains(&NupmReasonCode::ScriptPackage));
    }

    #[test]
    fn t08_custom_build() {
        let (a, _) = assess_source_root(&pkg("rejected/custom-with-build")).unwrap();
        assert_eq!(a.outcome, NupmOutcome::ManualMigrationRequired);
        assert_eq!(a.reason_codes, vec![NupmReasonCode::CustomBuildNu]);
        assert!(a.detected_features.has_build_script);
    }

    #[test]
    fn t09_module_scripts() {
        let (a, _) = assess_source_root(&pkg("rejected/module-with-scripts")).unwrap();
        assert_eq!(a.outcome, NupmOutcome::ManualMigrationRequired);
        assert!(a.reason_codes.contains(&NupmReasonCode::AuxiliaryScripts));
    }

    #[test]
    fn t10_external_deps() {
        let (a, _) = assess_source_root(&pkg("rejected/external-deps")).unwrap();
        assert_eq!(a.outcome, NupmOutcome::ManualMigrationRequired);
        assert_eq!(a.reason_codes, vec![NupmReasonCode::DeclaredDependencies]);
    }

    #[test]
    fn t11_missing_mod_nu() {
        let (a, _) = assess_source_root(&pkg("rejected/missing-mod-nu")).unwrap();
        assert_eq!(a.outcome, NupmOutcome::Unsupported);
        assert_eq!(a.reason_codes, vec![NupmReasonCode::MissingModuleEntry]);
    }

    #[test]
    fn t12_unknown_type() {
        let (a, _) = assess_source_root(&pkg("rejected/unknown-type")).unwrap();
        assert_eq!(a.outcome, NupmOutcome::Unsupported);
        assert_eq!(a.reason_codes, vec![NupmReasonCode::UnknownPackageType]);
    }

    #[test]
    fn installed_only_assessment_is_inspect_only() {
        let a = installed_only_assessment("minimal-module");
        assert_eq!(a.outcome, NupmOutcome::InspectOnly);
        assert_eq!(a.reason_codes, vec![NupmReasonCode::MetadataUnavailable]);
    }

    #[test]
    fn empty_version_is_unsupported_metadata() {
        use std::fs;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("pkg");
        fs::create_dir_all(root.join("m")).unwrap();
        fs::write(
            root.join("nupm.nuon"),
            br#"{ name: m, version: "", type: module }"#,
        )
        .unwrap();
        fs::write(root.join("m/mod.nu"), b"").unwrap();
        let (a, _) = assess_source_root(&root).unwrap();
        assert_eq!(a.outcome, NupmOutcome::Unsupported);
        assert_eq!(a.reason_codes, vec![NupmReasonCode::MissingRequiredKeys]);
    }

    #[test]
    fn custom_without_build_is_manual_migration() {
        let (a, _) = assess_source_root(&pkg("rejected/custom-without-build")).unwrap();
        assert_eq!(a.outcome, NupmOutcome::ManualMigrationRequired);
        assert_eq!(a.reason_codes, vec![NupmReasonCode::CustomBuildNu]);
    }

    #[test]
    fn missing_module_directory_is_unsupported() {
        let (a, _) = assess_source_root(&pkg("rejected/missing-module-dir")).unwrap();
        assert_eq!(a.outcome, NupmOutcome::Unsupported);
        assert_eq!(a.reason_codes, vec![NupmReasonCode::MissingModuleDirectory]);
    }

    #[test]
    fn missing_required_keys_is_unsupported() {
        let (a, _) = assess_source_root(&pkg("rejected/missing-required-keys")).unwrap();
        assert_eq!(a.outcome, NupmOutcome::Unsupported);
        assert_eq!(a.reason_codes, vec![NupmReasonCode::MissingRequiredKeys]);
    }

    #[test]
    fn unsupported_nuon_construct_is_unsupported() {
        let (a, _) = assess_source_root(&pkg("rejected/unsupported-nuon-construct")).unwrap();
        assert_eq!(a.outcome, NupmOutcome::Unsupported);
        assert_eq!(
            a.reason_codes,
            vec![NupmReasonCode::UnsupportedNuonConstruct]
        );
    }

    #[test]
    fn exhaustive_outcome_table_is_covered() {
        // This test ensures every NupmCompatibility variant maps to a known outcome.
        let cases: Vec<(NupmCompatibility, NupmOutcome)> = vec![
            (
                NupmCompatibility::ImportableModule,
                NupmOutcome::ImportableNow,
            ),
            (
                NupmCompatibility::DeferredScript,
                NupmOutcome::ManualMigrationRequired,
            ),
            (
                NupmCompatibility::UnsupportedDependencies,
                NupmOutcome::ManualMigrationRequired,
            ),
            (
                NupmCompatibility::UnsupportedCustomBuild,
                NupmOutcome::ManualMigrationRequired,
            ),
            (NupmCompatibility::UnknownType, NupmOutcome::Unsupported),
            (NupmCompatibility::InvalidMetadata, NupmOutcome::Unsupported),
            (
                NupmCompatibility::UnsafeFilesystemLayout,
                NupmOutcome::Unsupported,
            ),
        ];
        for (compat, expected) in cases {
            let a = build_assessment(Path::new("/dummy"), compat, None);
            assert_eq!(a.outcome, expected, "outcome mismatch for {compat:?}");
        }
    }
}
