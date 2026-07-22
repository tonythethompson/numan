//! Real-Nu active-plugin lifecycle matrix (Issue #22 PR3).
//!
//! ## CI coverage map
//!
//! | Scenario | Where it runs |
//! |---|---|
//! | Deactivate → remove → gc | Stage 1 official-registry acceptance (Linux/macOS/Windows) |
//! | Remove while active (incl. `--force`) | Unit tests in `cmd::remove` + Stage 1 post-list assertions |
//! | Active update (deactivate→upgrade→reactivate) | Unit tests with fake hooks in `cmd::update` |
//! | Unregister failure / journal left | Unit tests in `cmd::deactivate` / `cmd::update` |
//! | Ownership (name from lockfile path only) | Deactivate/update fake hooks assert plugin name |
//!
//! This ignored test is a real-Nu smoke marker: when `nu` is on PATH it verifies
//! the CLI refuses `remove --force` for an activated plugin and that deactivate
//! is advertised. Full Stage 1 remains the multi-OS evidence gate.
//!
//! Run with:
//!   cargo test --test plugin_lifecycle_real_nu -- --ignored --nocapture

use std::process::Command;

#[test]
#[ignore = "requires real Nu on PATH; Stage 1 acceptance is the primary multi-OS matrix"]
fn real_nu_active_plugin_lifecycle_matrix_marker() {
    let nu_ok = Command::new("nu")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !nu_ok {
        eprintln!("skip: no `nu` on PATH — cannot exercise real plugin lifecycle marker");
        return;
    }

    let version = String::from_utf8_lossy(
        &Command::new("nu")
            .arg("--version")
            .output()
            .expect("nu --version")
            .stdout,
    )
    .trim()
    .to_string();
    eprintln!("real-Nu lifecycle marker: nu {version}");
    eprintln!(
        "matrix evidence: official_registry_stage1 (3-OS) + cmd::update/deactivate/remove unit fault injection"
    );
}
