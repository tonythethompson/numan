//! Stage 1 orchestration is added with the ignored live test. This module owns
//! run-directory creation and durable failure finalization shared by ordinary
//! infrastructure tests and the live runner.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use uuid::Uuid;

use super::model::{
    utc_unix_ms, AcceptanceFailure, ChildEnvironment, RunRecord, RunSummary, Stage1Config,
    EVIDENCE_SCHEMA_VERSION,
};

pub struct AcceptanceRun {
    pub config: Stage1Config,
    pub numan_binary: PathBuf,
    pub run_id: String,
    pub run_dir: PathBuf,
    pub root: PathBuf,
    pub home: PathBuf,
    pub evidence: PathBuf,
    pub environment: ChildEnvironment,
    pub record: RunRecord,
}

impl AcceptanceRun {
    pub fn new(config: Stage1Config, numan_binary: PathBuf) -> Result<Self> {
        let now = utc_unix_ms();
        let uuid = Uuid::new_v4().simple().to_string();
        let run_id = format!("{now}-{}", &uuid[..8]);
        let base = absolute_path(&config.output_base)?;
        let run_dir = base.join(&run_id);
        let root = run_dir.join("root");
        let home = run_dir.join("home");
        let evidence = run_dir.join("evidence");
        std::fs::create_dir_all(&home)?;
        std::fs::create_dir_all(&evidence)?;
        let environment = ChildEnvironment::isolated(&home)?;
        let record = RunRecord {
            schema_version: EVIDENCE_SCHEMA_VERSION,
            run_id: run_id.clone(),
            started_utc_ms: now,
            finished_utc_ms: None,
            status: "running".to_string(),
            config: config.clone(),
            numan_binary: numan_binary.to_string_lossy().into_owned(),
            root: root.to_string_lossy().into_owned(),
            home: home.to_string_lossy().into_owned(),
            evidence: evidence.to_string_lossy().into_owned(),
            environment: environment.clone(),
        };
        write_json(&evidence.join("run.json"), &record)?;
        Ok(Self {
            config,
            numan_binary,
            run_id,
            run_dir,
            root,
            home,
            evidence,
            environment,
            record,
        })
    }

    pub fn execute(&mut self) -> Result<RunSummary, AcceptanceFailure> {
        Err(AcceptanceFailure {
            failed_step: "not-implemented".to_string(),
            arguments: Vec::new(),
            exit_code: None,
            timed_out: false,
            assertion_errors: vec!["live lifecycle is not wired yet".to_string()],
            stdout_path: String::new(),
            stderr_path: String::new(),
            evidence_directory: self.evidence.to_string_lossy().into_owned(),
        })
    }

    pub fn finalize(&mut self, summary: &RunSummary) -> Result<()> {
        self.record.status = summary.status.clone();
        self.record.finished_utc_ms = Some(utc_unix_ms());
        write_json(&self.evidence.join("run.json"), &self.record)?;
        write_json(&self.evidence.join("summary.json"), summary)?;
        std::fs::write(self.evidence.join("summary.md"), render_summary(summary))?;
        Ok(())
    }
}

pub fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(value)?;
    std::fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))
}

fn render_summary(summary: &RunSummary) -> String {
    let mut output = format!(
        "# Official registry Stage 1 acceptance\n\n- Run: `{}`\n- Status: **{}**\n- Package: `{}`\n- Resolved version: `{}`\n\n## Steps\n",
        summary.run_id,
        summary.status,
        summary.package_id,
        summary.resolved_version.as_deref().unwrap_or("unresolved")
    );
    for step in &summary.steps {
        output.push_str(&format!(
            "\n- `{}`: {} (exit {:?}, timeout {})",
            step.step,
            if step.passed { "passed" } else { "failed" },
            step.exit_code,
            step.timed_out
        ));
        for error in &step.assertion_errors {
            output.push_str(&format!("\n  - {error}"));
        }
    }
    output.push('\n');
    output
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}
