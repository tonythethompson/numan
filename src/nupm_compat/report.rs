use std::io::Write;
use std::path::PathBuf;

use super::classify::NupmCompatibility;
use super::drift::{DriftReport, DriftStatus};
use super::metadata::ParsedMetadata;
use super::schema::COMPAT_SCHEMA_VERSION;

#[derive(Debug, Clone)]
pub struct NupmStatusReport {
    pub nupm_home: Option<PathBuf>,
    pub home_not_configured: bool,
    pub modules_dir_present: bool,
    pub scripts_dir_present: bool,
    pub import_eligible: usize,
    pub rejected_source: usize,
    pub installed_only: usize,
    pub script_entries: usize,
    pub unsafe_entries: usize,
    pub numan_nupm_imports: usize,
    pub source_drift_count: usize,
    pub name_overlap_count: usize,
}

#[derive(Debug, Clone)]
pub struct SourceRootEntry {
    pub source_path: PathBuf,
    pub compatibility: NupmCompatibility,
    pub metadata: Option<ParsedMetadata>,
}

#[derive(Debug, Clone)]
pub struct InstalledOnlyEntry {
    pub name: String,
    pub path: PathBuf,
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
        writeln!(out, "Source roots classified:")?;
        writeln!(out, "  import-eligible: {}", report.import_eligible)?;
        writeln!(out, "  rejected: {}", report.rejected_source)?;
        writeln!(out)?;
        writeln!(
            out,
            "Installed-only module directories: {} (metadata unavailable; not import-eligible)",
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

pub fn format_inspection_report(
    report: &NupmInspectionReport,
    out: &mut dyn Write,
) -> std::io::Result<()> {
    for c in &report.candidates {
        format_candidate(&c.entry, out)?;
        writeln!(out)?;
    }
    for inst in &report.installed_only {
        writeln!(out, "{} (installed-only)", inst.name)?;
        writeln!(out, "  Source:       {}", inst.path.display())?;
        writeln!(out, "  Metadata:     unavailable")?;
        writeln!(
            out,
            "  Eligible:     no (metadata unavailable; not eligible for Numan import)"
        )?;
        writeln!(out)?;
    }
    Ok(())
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
        "  Compatibility: {}",
        compatibility_label(entry.compatibility)
    )?;
    writeln!(
        out,
        "  Eligible:     {}",
        if entry.compatibility == NupmCompatibility::ImportableModule {
            "yes"
        } else {
            "no"
        }
    )?;
    if entry.compatibility == NupmCompatibility::ImportableModule {
        writeln!(
            out,
            "  Import:       numan nupm import {} --as owner/name [--yes]",
            entry.source_path.display()
        )?;
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

fn compatibility_label(c: NupmCompatibility) -> &'static str {
    match c {
        NupmCompatibility::ImportableModule => "ImportableModule",
        NupmCompatibility::DeferredScript => "DeferredScript",
        NupmCompatibility::UnsupportedCustomBuild => "UnsupportedCustomBuild",
        NupmCompatibility::UnsupportedDependencies => "UnsupportedDependencies",
        NupmCompatibility::InvalidMetadata => "InvalidMetadata",
        NupmCompatibility::UnsafeFilesystemLayout => "UnsafeFilesystemLayout",
        NupmCompatibility::UnknownType => "UnknownType",
    }
}
