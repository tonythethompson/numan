//! Real-Nu plugin deactivate path (Issue #22 PR2).
//!
//! Documents the production path: activate a plugin, then deactivate it with
//! the real Nu `plugin rm` seam. Ignored by default because it needs a Nu
//! binary on PATH and an installed plugin fixture.
//!
//! Run with:
//!   cargo test --test plugin_deactivate_real_nu -- --ignored --nocapture
//!
//! Preconditions: `nu --version` succeeds. Without Nu, this test returns early
//! (same pattern as other real-Nu acceptance tests).

use std::process::Command;

#[test]
#[ignore = "requires real Nu on PATH and an activated plugin fixture"]
fn real_nu_plugin_deactivate_path_documented() {
    let nu_ok = Command::new("nu")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !nu_ok {
        eprintln!("skip: no `nu` on PATH — cannot exercise real plugin deactivate");
        return;
    }

    // Full lifecycle lives in Stage 1 acceptance
    // (`official_registry_stage1` → list → deactivate → remove → gc).
    // This ignored test exists so CI/docs can point at a dedicated real-Nu
    // deactivate marker until a dedicated plugin binary fixture is checked in.
    eprintln!(
        "real-Nu deactivate: use Stage 1 acceptance or manually \
         `numan activate <plugin> --yes` then `numan deactivate <plugin> --yes`"
    );
}
