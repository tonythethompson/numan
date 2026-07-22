//! Real-Nu plugin deactivate coverage pointer (Issue #22 PR2).
//!
//! This crate intentionally does **not** ship a green no-op ignored test.
//! Authoritative real-Nu deactivate evidence is the Stage 1 acceptance harness:
//!
//! ```text
//! cargo test --locked --test official_registry_stage1 stage1_official_registry -- --ignored --nocapture --test-threads=1
//! ```
//!
//! See `docs/acceptance/official-registry-stage1.md`.

#[test]
fn stage1_is_authoritative_real_nu_deactivate_gate() {
    let doc = include_str!("../docs/acceptance/official-registry-stage1.md");
    assert!(
        doc.contains("deactivate → remove → gc") || doc.contains("deactivate"),
        "Stage 1 docs must describe deactivate as part of the lifecycle gate"
    );
    assert!(
        doc.contains("official_registry_stage1") || doc.contains("Stage 1"),
        "Stage 1 docs must identify the acceptance harness"
    );
}
