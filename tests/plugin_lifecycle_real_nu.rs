//! Real-Nu active-plugin lifecycle matrix (Issue #22 PR3).
//!
//! ## CI coverage map (honest)
//!
//! | Scenario | Where it runs today |
//! |---|---|
//! | Deactivate → remove → gc | Stage 1 official-registry acceptance (Windows x86_64) |
//! | Remove while active (incl. `--force`) | Unit tests in `cmd::remove` + Stage 1 post-list assertions |
//! | Active update (deactivate→upgrade→reactivate) | Unit tests with fake hooks in `cmd::update` only |
//! | Unregister failure / journal left | Unit tests in `cmd::deactivate` / `cmd::update` |
//! | Ownership (path + lockfile identity) | Deactivate/update fake hooks assert absolute binary path |
//!
//! ## TODO: required before default-on (not green yet)
//!
//! - [ ] Real-Nu active **update** e2e on Linux/macOS/Windows
//! - [ ] Failed upgrade after deactivate restores previous activation (real Nu)
//! - [ ] Unregister failure leaves activation + journals (real Nu)
//! - [ ] Reactivate failure after successful upgrade leaves recovery guidance (real Nu)
//! - [ ] Stale/mismatched Nu identity refuses update (preserves activation; real Nu)
//! - [ ] Full fault-injection matrix documented and green on 3 OS
//!
//! This ignored test is a real-Nu smoke marker only. It does **not** claim the
//! matrix above is green. Stage 1 remains the Windows/x86_64 evidence gate for
//! deactivate→remove; active update stays opt-in until the TODO list closes.
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
        eprintln!("skip: no `nu` on PATH: cannot exercise real plugin lifecycle marker");
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
        "matrix status: Stage 1 deactivate→remove (Windows x86_64) green; active update real-Nu e2e + fault matrix still TODO (opt-in)"
    );
}
