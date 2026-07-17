use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Instant;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use wait_timeout::ChildExt;

use super::model::{
    utc_unix_ms, ChildEnvironment, CommandOutcome, CommandSpec, EVIDENCE_SCHEMA_VERSION,
};

struct CapturedStream {
    bytes: Vec<u8>,
    sha256: String,
}

pub fn run_command(spec: &CommandSpec, environment: &ChildEnvironment) -> Result<CommandOutcome> {
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.arguments)
        .env_clear()
        .envs(&environment.variables)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let started_utc_ms = utc_unix_ms();
    let started = Instant::now();
    let mut child = command.spawn().with_context(|| {
        format!(
            "failed to spawn acceptance command {}",
            spec.program.display()
        )
    })?;
    let stdout = child.stdout.take().context("child stdout was not piped")?;
    let stderr = child.stderr.take().context("child stderr was not piped")?;
    let stdout_thread = thread::spawn(move || capture_stream(stdout));
    let stderr_thread = thread::spawn(move || capture_stream(stderr));

    let (status, timed_out) = match child.wait_timeout(spec.timeout())? {
        Some(status) => (status, false),
        None => {
            let _ = child.kill();
            (
                child.wait().context("failed to reap timed-out child")?,
                true,
            )
        }
    };
    let stdout = stdout_thread
        .join()
        .map_err(|_| anyhow::anyhow!("stdout reader thread panicked"))??;
    let stderr = stderr_thread
        .join()
        .map_err(|_| anyhow::anyhow!("stderr reader thread panicked"))??;

    Ok(CommandOutcome {
        schema_version: EVIDENCE_SCHEMA_VERSION,
        started_utc_ms,
        finished_utc_ms: utc_unix_ms(),
        duration_ms: started.elapsed().as_millis(),
        exit_code: status.code(),
        timed_out,
        stdout_bytes: stdout.bytes.len().try_into().unwrap_or(u64::MAX),
        stderr_bytes: stderr.bytes.len().try_into().unwrap_or(u64::MAX),
        stdout_sha256: stdout.sha256,
        stderr_sha256: stderr.sha256,
        stdout: stdout.bytes,
        stderr: stderr.bytes,
    })
}

pub fn streaming_sha256<R: Read>(mut reader: R) -> Result<(String, u64)> {
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut bytes = 0u64;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        bytes = bytes.saturating_add(read.try_into().unwrap_or(u64::MAX));
    }
    Ok((hex::encode(hasher.finalize()), bytes))
}

fn capture_stream<R: Read>(mut reader: R) -> Result<CapturedStream> {
    let mut bytes = Vec::new();
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        bytes.write_all(&buffer[..read])?;
    }
    Ok(CapturedStream {
        bytes,
        sha256: hex::encode(hasher.finalize()),
    })
}
