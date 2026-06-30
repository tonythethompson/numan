use std::io::Write;
use std::path::Path;

use anyhow::{bail, Result};

use crate::nupm_compat::{
    format_inspection_report, format_status_report, inspect_path, resolve_nupm_home,
    scan_nupm_home, NupmCandidateReport, NupmCompatibility, NupmHomeResolution,
    NupmInspectionReport, NupmStatusReport,
};
use crate::state::lockfile::Lockfile;

#[derive(clap::Parser)]
pub struct NupmArgs {
    #[command(subcommand)]
    pub command: NupmCommands,
}

#[derive(clap::Subcommand)]
pub enum NupmCommands {
    Status(StatusArgs),
    Inspect(InspectArgs),
}

#[derive(clap::Parser)]
pub struct StatusArgs {
    #[arg(long)]
    pub nupm_home: Option<std::path::PathBuf>,
}

#[derive(clap::Parser)]
pub struct InspectArgs {
    /// Inspect all discoverable candidates under nupm home
    #[arg(long, group = "inspect_target")]
    pub all: bool,

    /// Package source path (mutually exclusive with --all)
    #[arg(value_name = "PATH", group = "inspect_target")]
    pub path: Option<std::path::PathBuf>,

    #[arg(long)]
    pub nupm_home: Option<std::path::PathBuf>,
}

pub fn execute(args: &NupmArgs, numan_root: &Path, out: &mut dyn Write) -> Result<()> {
    match &args.command {
        NupmCommands::Status(a) => run_status(a, numan_root, out),
        NupmCommands::Inspect(a) => run_inspect(a, out),
    }
}

fn run_status(args: &StatusArgs, numan_root: &Path, out: &mut dyn Write) -> Result<()> {
    let lockfile = Lockfile::load(numan_root)?;
    let numan_nupm_imports = lockfile.count_nupm_imports();

    match resolve_nupm_home(args.nupm_home.as_deref())? {
        NupmHomeResolution::NotConfigured => {
            let report = NupmStatusReport {
                nupm_home: None,
                home_not_configured: true,
                modules_dir_present: false,
                scripts_dir_present: false,
                import_eligible: 0,
                rejected_source: 0,
                installed_only: 0,
                script_entries: 0,
                unsafe_entries: 0,
                numan_nupm_imports,
            };
            format_status_report(&report, out)?;
            Ok(())
        }
        NupmHomeResolution::Found(home) => {
            let scan = scan_nupm_home(&home)?;
            let (import_eligible, rejected_source) = count_source(&scan.source_roots);
            let report = NupmStatusReport {
                nupm_home: Some(home.clone()),
                home_not_configured: false,
                modules_dir_present: home.join("modules").is_dir(),
                scripts_dir_present: home.join("scripts").is_dir(),
                import_eligible,
                rejected_source,
                installed_only: scan.installed_only.len(),
                script_entries: scan.script_entries,
                unsafe_entries: scan.unsafe_entries,
                numan_nupm_imports,
            };
            format_status_report(&report, out)?;
            Ok(())
        }
    }
}

fn run_inspect(args: &InspectArgs, out: &mut dyn Write) -> Result<()> {
    if args.all {
        if args.nupm_home.is_none() {
            // --nupm-home optional if NUPM_HOME set; resolve handles both
        }
        let home = match resolve_nupm_home(args.nupm_home.as_deref())? {
            NupmHomeResolution::Found(p) => p,
            NupmHomeResolution::NotConfigured => {
                bail!(
                    "No nupm home was supplied.\n\n\
                     Pass --nupm-home <path> or set NUPM_HOME for inspect --all.\n\
                     Numan will not guess nupm's installation location."
                );
            }
        };
        let scan = scan_nupm_home(&home)?;
        let candidates = scan
            .source_roots
            .into_iter()
            .map(|entry| NupmCandidateReport { entry })
            .collect();
        let report = NupmInspectionReport {
            candidates,
            installed_only: scan.installed_only,
        };
        format_inspection_report(&report, out)?;
        Ok(())
    } else if let Some(path) = &args.path {
        if args.nupm_home.is_some() {
            bail!("--nupm-home cannot be used with inspect <PATH>; use inspect --all --nupm-home instead");
        }
        let entry = inspect_path(path)?;
        let report = NupmInspectionReport {
            candidates: vec![NupmCandidateReport { entry }],
            installed_only: vec![],
        };
        format_inspection_report(&report, out)?;
        Ok(())
    } else {
        bail!("inspect requires either <PATH> or --all")
    }
}

fn count_source(roots: &[crate::nupm_compat::SourceRootEntry]) -> (usize, usize) {
    let mut eligible = 0usize;
    let mut rejected = 0usize;
    for r in roots {
        if r.compatibility == NupmCompatibility::ImportableModule {
            eligible += 1;
        } else {
            rejected += 1;
        }
    }
    (eligible, rejected)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_not_configured_exits_ok_via_report() {
        let mut buf = Vec::new();
        let args = NupmArgs {
            command: NupmCommands::Status(StatusArgs { nupm_home: None }),
        };
        // Temporarily clear NUPM_HOME for test
        let prev = std::env::var_os("NUPM_HOME");
        std::env::remove_var("NUPM_HOME");
        let root = tempfile::tempdir().unwrap();
        execute(&args, root.path(), &mut buf).unwrap();
        if let Some(p) = prev {
            std::env::set_var("NUPM_HOME", p);
        }
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("not configured"));
    }
}
