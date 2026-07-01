pub mod assessment;
pub mod classify;
pub mod discovery;
pub mod drift;
pub mod import;
pub mod metadata;
pub mod report;
pub mod schema;
pub mod walk;

pub use assessment::{
    assess_source_root, installed_only_assessment, DetectedFeatures, NupmAssessment, NupmOutcome,
    NupmReasonCode, NupmRecommendedAction,
};
pub use classify::{classify_source_root, find_source_root, NupmCompatibility};
pub use discovery::{inspect_path, resolve_nupm_home, scan_nupm_home, NupmHomeResolution};
pub use drift::{compare_import, count_drifted_imports, DriftReport, DriftStatus};
pub use import::{
    import_manifest_with_runner, import_module, import_module_with_runner, ImportManifestResult,
    ImportResult,
};
pub use metadata::{
    parse_metadata, read_metadata_limited, BehaviorFlags, MetadataError, ParsedMetadata,
};
pub use report::{
    format_drift_report, format_inspection_json, format_inspection_report, format_status_json,
    format_status_report, InstalledOnlyEntry, NupmCandidateReport, NupmInspectJson,
    NupmInspectionReport, NupmStatusJson, NupmStatusReport, SourceRootEntry, SourceRootJson,
};
