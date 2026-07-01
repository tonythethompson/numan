use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::assessment::{
    DetectedFeatures, NupmAssessment, NupmOutcome, NupmReasonCode, NupmRecommendedAction,
};
use super::drift::{DriftReport, DriftStatus};
use super::metadata::ParsedMetadata;
use super::schema::COMPAT_SCHEMA_VERSION;

pub const MIGRATION_JSON_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct NupmStatusReport {
    pub nupm_home: Option<PathBuf>,
    pub home_not_configured: bool,
    pub modules_dir_present: bool,
    pub scripts_dir_present: bool,
    pub import_eligible: usize,
    pub inspect_only: usize,
    pub manual_migration_required: usize,
    pub unsupported: usize,
    pub installed_only: usize,
    pub script_entries: usize,
    pub unsafe_entries: usize,
    pub numan_nupm_imports: usize,
    pub source_drift_count: usize,
    pub name_overlap_count: usize,
    pub source_roots: Vec<SourceRootEntry>,
    pub installed_only_entries: Vec<InstalledOnlyEntry>,
}

#[derive(Debug, Clone)]
pub struct SourceRootEntry {
    pub source_path: PathBuf,
    pub metadata: Option<ParsedMetadata>,
    pub assessment: NupmAssessment,
}

#[derive(Debug, Clone)]
pub struct InstalledOnlyEntry {
    pub name: String,
    pub path: PathBuf,
    pub assessment: NupmAssessment,
}

#[derive(Debug, Clone)]
pub struct NupmCandidateReport {
    pub entry: SourceRootEntry,
}

#[derive(Debug, Clone)]
pub struct NupmInspectionReport {
    pub candidates: Vec<NupmCandidateReport>,
    pub installed_only: Vec<InstalledOnlyEntry>,
}

// JSON envelope types

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NupmStatusJson {
    pub schema_version: u32,
    pub compat_schema_version: u32,
    pub command: String,
    pub nupm_home: Option<String>,
    pub home_not_configured: bool,
    pub modules_dir_present: bool,
    pub scripts_dir_present: bool,
    pub counts: BTreeMap<String, usize>,
    pub source_roots: Vec<SourceRootJson>,
    pub installed_only: Vec<InstalledOnlyJson>,
    pub numan_nupm_imports: usize,
    pub source_drift_count: usize,
    pub name_overlap_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NupmInspectJson {
    pub schema_version: u32,
    pub compat_schema_version: u32,
    pub command: String,
    pub nupm_home: Option<String>,
    pub candidates: Vec<SourceRootJson>,
    pub installed_only: Vec<InstalledOnlyJson>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SourceRootJson {
    pub name: String,
    pub source_path: String,
    pub version: Option<String>,
    pub package_type: Option<String>,
    pub outcome: NupmOutcome,
    pub reason_codes: Vec<NupmReasonCode>,
    pub recommended_action: NupmRecommendedAction,
    pub detected_features: DetectedFeatures,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct InstalledOnlyJson {
    pub name: String,
    pub path: String,
    pub outcome: NupmOutcome,
    pub reason_codes: Vec<NupmReasonCode>,
    pub recommended_action: NupmRecommendedAction,
    pub detected_features: DetectedFeatures,
}

pub fn format_status_report(report: &NupmStatusReport, out: &mut dyn Write) -> std::io::Result<()> {
    if report.home_not_configured {
        writeln!(out, "nupm home: not configured")?;
        writeln!(
            out,
            "\nPass --nupm-home <path> or set NUPM_HOME for nupm discovery.\n\
             Numan will not guess nupm's installation location."
        )?;
    } else if let Some(home) = &report.nupm_home {
        writeln!(out, "nupm home: {}", home.display())?;
        writeln!(
            out,
            "modules dir: {}",
            if report.modules_dir_present {
                "present"
            } else {
                "absent"
            }
        )?;
        writeln!(
            out,
            "scripts dir: {}",
            if report.scripts_dir_present {
                "present"
            } else {
                "absent"
            }
        )?;
        writeln!(out)?;
        writeln!(out, "Migration outcomes:")?;
        writeln!(
            out,
            "  importable_now:              {}",
            report.import_eligible
        )?;
        writeln!(
            out,
            "  inspect_only:                {}",
            report.inspect_only
        )?;
        writeln!(
            out,
            "  manual_migration_required:   {}",
            report.manual_migration_required
        )?;
        writeln!(out, "  unsupported:                 {}", report.unsupported)?;
        writeln!(out)?;
        writeln!(
            out,
            "Installed-only module directories: {} (metadata unavailable)",
            report.installed_only
        )?;
        writeln!(out, "Script entries: {}", report.script_entries)?;
        writeln!(out, "Unsafe/unreadable entries: {}", report.unsafe_entries)?;
        if report.name_overlap_count > 0 {
            writeln!(
                out,
                "Name overlap warnings: {} (nupm source name matches a different installed Numan module)",
                report.name_overlap_count
            )?;
        }
        writeln!(out)?;
        for entry in &report.source_roots {
            if entry.assessment.outcome != NupmOutcome::ImportableNow {
                format_next_action(entry, out)?;
            }
        }
        for entry in &report.installed_only_entries {
            format_next_action_installed_only(entry, out)?;
        }
    }
    writeln!(
        out,
        "Numan nupm imports (lockfile): {}",
        report.numan_nupm_imports
    )?;
    writeln!(out, "Source drift (imports): {}", report.source_drift_count)?;
    writeln!(out, "(compat-schema-v{COMPAT_SCHEMA_VERSION})")?;
    Ok(())
}

pub fn format_status_json(report: &NupmStatusReport) -> serde_json::Result<String> {
    let mut counts = BTreeMap::new();
    counts.insert("importable_now".to_string(), report.import_eligible);
    counts.insert("inspect_only".to_string(), report.inspect_only);
    counts.insert(
        "manual_migration_required".to_string(),
        report.manual_migration_required,
    );
    counts.insert("unsupported".to_string(), report.unsupported);
    counts.insert("installed_only".to_string(), report.installed_only);
    counts.insert("script_entries".to_string(), report.script_entries);
    counts.insert("unsafe_entries".to_string(), report.unsafe_entries);

    let nupm_home = report.nupm_home.as_ref().map(|p| p.display().to_string());

    let value = NupmStatusJson {
        schema_version: MIGRATION_JSON_SCHEMA_VERSION,
        compat_schema_version: COMPAT_SCHEMA_VERSION,
        command: "status".to_string(),
        nupm_home,
        home_not_configured: report.home_not_configured,
        modules_dir_present: report.modules_dir_present,
        scripts_dir_present: report.scripts_dir_present,
        counts,
        source_roots: report
            .source_roots
            .iter()
            .map(source_root_to_json)
            .collect(),
        installed_only: report
            .installed_only_entries
            .iter()
            .map(installed_only_to_json)
            .collect(),
        numan_nupm_imports: report.numan_nupm_imports,
        source_drift_count: report.source_drift_count,
        name_overlap_count: report.name_overlap_count,
    };
    serde_json::to_string_pretty(&value)
}

pub fn format_inspection_report(
    report: &NupmInspectionReport,
    out: &mut dyn Write,
) -> std::io::Result<()> {
    for c in &report.candidates {
        format_candidate(&c.entry, out)?;
        writeln!(out)?;
    }
    for inst in &report.installed_only {
        format_candidate_installed_only(inst, out)?;
        writeln!(out)?;
    }
    Ok(())
}

pub fn format_inspection_json(
    report: &NupmInspectionReport,
    nupm_home: Option<&PathBuf>,
) -> serde_json::Result<String> {
    let nupm_home = nupm_home.map(|p| p.display().to_string());
    let value = NupmInspectJson {
        schema_version: MIGRATION_JSON_SCHEMA_VERSION,
        compat_schema_version: COMPAT_SCHEMA_VERSION,
        command: "inspect".to_string(),
        nupm_home,
        candidates: report
            .candidates
            .iter()
            .map(|c| source_root_to_json(&c.entry))
            .collect(),
        installed_only: report
            .installed_only
            .iter()
            .map(installed_only_to_json)
            .collect(),
    };
    serde_json::to_string_pretty(&value)
}

fn source_root_to_json(entry: &SourceRootEntry) -> SourceRootJson {
    let name = entry
        .metadata
        .as_ref()
        .map(|m| m.name.clone())
        .unwrap_or_else(|| "?".to_string());
    SourceRootJson {
        name,
        source_path: entry.source_path.display().to_string(),
        version: entry.metadata.as_ref().map(|m| m.version.clone()),
        package_type: entry.metadata.as_ref().map(|m| m.package_type.clone()),
        outcome: entry.assessment.outcome,
        reason_codes: entry.assessment.reason_codes.clone(),
        recommended_action: entry.assessment.recommended_action,
        detected_features: entry.assessment.detected_features.clone(),
    }
}

fn installed_only_to_json(entry: &InstalledOnlyEntry) -> InstalledOnlyJson {
    InstalledOnlyJson {
        name: entry.name.clone(),
        path: entry.path.display().to_string(),
        outcome: entry.assessment.outcome,
        reason_codes: entry.assessment.reason_codes.clone(),
        recommended_action: entry.assessment.recommended_action,
        detected_features: entry.assessment.detected_features.clone(),
    }
}

fn format_candidate(entry: &SourceRootEntry, out: &mut dyn Write) -> std::io::Result<()> {
    let name = entry
        .metadata
        .as_ref()
        .map(|m| m.name.as_str())
        .unwrap_or("?");
    writeln!(out, "{name}")?;
    writeln!(out, "  Source:       {}", entry.source_path.display())?;
    if let Some(meta) = &entry.metadata {
        writeln!(out, "  Type:         {}", meta.package_type)?;
        writeln!(out, "  Version:      {}", meta.version)?;
    }
    writeln!(
        out,
        "  Outcome:      {}",
        outcome_label(entry.assessment.outcome)
    )?;
    writeln!(
        out,
        "  Reasons:      {}",
        reason_codes_label(&entry.assessment.reason_codes)
    )?;
    writeln!(
        out,
        "  Action:       {}",
        action_label(entry.assessment.recommended_action)
    )?;
    if entry.assessment.outcome == NupmOutcome::ImportableNow {
        writeln!(
            out,
            "  Import:       numan nupm import {} --as owner/name [--yes]",
            entry.source_path.display()
        )?;
    }
    Ok(())
}

fn format_candidate_installed_only(
    entry: &InstalledOnlyEntry,
    out: &mut dyn Write,
) -> std::io::Result<()> {
    writeln!(out, "{} (installed-only)", entry.name)?;
    writeln!(out, "  Source:       {}", entry.path.display())?;
    writeln!(out, "  Metadata:     unavailable")?;
    writeln!(
        out,
        "  Outcome:      {}",
        outcome_label(entry.assessment.outcome)
    )?;
    writeln!(
        out,
        "  Reasons:      {}",
        reason_codes_label(&entry.assessment.reason_codes)
    )?;
    writeln!(
        out,
        "  Action:       {}",
        action_label(entry.assessment.recommended_action)
    )?;
    Ok(())
}

fn format_next_action(entry: &SourceRootEntry, out: &mut dyn Write) -> std::io::Result<()> {
    let name = entry
        .metadata
        .as_ref()
        .map(|m| m.name.as_str())
        .unwrap_or("?");
    writeln!(
        out,
        "  {name}: {outcome} ({reasons}) -> {action}",
        outcome = outcome_label(entry.assessment.outcome),
        reasons = reason_codes_label(&entry.assessment.reason_codes),
        action = action_label(entry.assessment.recommended_action)
    )?;
    Ok(())
}

fn format_next_action_installed_only(
    entry: &InstalledOnlyEntry,
    out: &mut dyn Write,
) -> std::io::Result<()> {
    writeln!(
        out,
        "  {}: {} ({}) -> {}",
        entry.name,
        outcome_label(entry.assessment.outcome),
        reason_codes_label(&entry.assessment.reason_codes),
        action_label(entry.assessment.recommended_action)
    )?;
    Ok(())
}

fn outcome_label(outcome: NupmOutcome) -> &'static str {
    match outcome {
        NupmOutcome::ImportableNow => "importable_now",
        NupmOutcome::InspectOnly => "inspect_only",
        NupmOutcome::ManualMigrationRequired => "manual_migration_required",
        NupmOutcome::Unsupported => "unsupported",
    }
}

fn reason_codes_label(reasons: &[NupmReasonCode]) -> String {
    if reasons.is_empty() || reasons == [NupmReasonCode::None] {
        return "none".to_string();
    }
    reasons
        .iter()
        .filter(|r| **r != NupmReasonCode::None)
        .map(|r| r.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn action_label(action: NupmRecommendedAction) -> &'static str {
    match action {
        NupmRecommendedAction::Import => "import with 'numan nupm import <path> --as owner/name'",
        NupmRecommendedAction::Inspect => "inspect source and locate metadata",
        NupmRecommendedAction::ManualMigration => {
            "migrate manually after reviewing package behavior"
        }
        NupmRecommendedAction::RepairSource => "repair source package before retrying",
    }
}

pub fn format_drift_report(report: &DriftReport, out: &mut dyn Write) -> std::io::Result<()> {
    writeln!(out, "Package: {}", report.package_id)?;
    writeln!(out, "Status:  {}", drift_status_label(&report.status))?;
    if !report.recorded_source.as_os_str().is_empty() {
        writeln!(out, "Source:  {}", report.recorded_source.display())?;
    }
    if let Some(rev) = &report.installed_revision_id {
        writeln!(out, "Installed revision_id: {rev}")?;
    }
    if !report.recorded_metadata_sha256.is_empty() {
        writeln!(
            out,
            "Recorded metadata sha256:   {}",
            report.recorded_metadata_sha256
        )?;
        if let Some(live) = &report.live_metadata_sha256 {
            writeln!(out, "Live metadata sha256:       {live}")?;
        }
    }
    if !report.recorded_source_payload_sha256.is_empty() {
        writeln!(
            out,
            "Recorded source payload sha256:   {}",
            report.recorded_source_payload_sha256
        )?;
        if let Some(live) = &report.live_source_payload_sha256 {
            writeln!(out, "Live source payload sha256:       {live}")?;
        }
    }
    if let DriftStatus::CannotCompare { reason } = &report.status {
        writeln!(out, "Reason:  {reason}")?;
    }
    Ok(())
}

fn drift_status_label(status: &DriftStatus) -> &'static str {
    match status {
        DriftStatus::Unchanged => "Unchanged",
        DriftStatus::SourceMissing => "SourceMissing",
        DriftStatus::MetadataChanged => "MetadataChanged",
        DriftStatus::PayloadChanged => "PayloadChanged",
        DriftStatus::UnsafeSourceTreeChange => "UnsafeSourceTreeChange",
        DriftStatus::CannotCompare { .. } => "CannotCompare",
    }
}
