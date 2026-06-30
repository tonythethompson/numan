# Numan — Nushell Package Manager

## Overview
Numan is a cross-platform, production-grade Nushell package manager CLI written in Rust. It handles plugins, modules, scripts, and completions with verified artifacts, compatibility resolution, lockfiles, rollback, and interoperability with nupm.

## Build & Test
```bash
# Build
cargo build

# Run
cargo run -- search <query>
cargo run -- info <owner/name>
cargo run -- list
cargo run -- nupm status --nupm-home <path>
cargo run -- nupm inspect <package-path>

# Test (245+ tests)
cargo test

# Test single module
cargo test core::platform
cargo test core::package
cargo test core::resolve
cargo test state::lockfile
cargo test cmd::activate
```

## Project Structure
```
src/
  main.rs              — CLI entry point (clap-based)
  config.rs            — Config load/save, root resolution
  core/
    platform.rs        — OS/arch/target detection
    package.rs         — ScopedId, Package, VersionEntry, Artifact types
    nu_version.rs      — Nu version detection and constraint matching
    registry.rs        — Registry index load/search/verify
    trust.rs           — Ed25519 trust store and signature verification
    integrity.rs       — SHA256 compute and verify
    resolve.rs         — Version resolution with strict plugin constraints
  cmd/
    search.rs          — Search subcommand
    info.rs            — Info subcommand
    list.rs            — List subcommand
    registry.rs        — Registry management subcommands
    activate.rs        — Plugin + module activation (Phase 3 & 4); public entry: execute_with_candidate_runner
    deactivate.rs      — Module deactivation: full (delete managed file) and partial (regenerate) (Phase 4)
    update.rs          — `numan update [--check] [pkg]`: detect and apply registry version upgrades (Phase 5)
    remove.rs          — `numan remove [--force] <pkg>`: remove from lockfile + delete payload (Phase 5)
    gc.rs              — `numan gc [--dry-run]`: delete orphaned payload directories (Phase 5)
    nupm.rs            — `numan nupm status|inspect|import|diff`: nupm discovery + import + drift (Phase 6.1–6.3)
  install/
    download.rs        — HTTP download with progress
    transaction.rs     — Full install flow (resolve→download→verify→extract→lockfile)
  state/
    lockfile.rs        — Lockfile v2: PluginActivation, ModuleActivation, revision_id, payload_sha256, compute_revision_id()
    journal.rs         — Plugin pending-activation journal for crash recovery
    autoload_journal.rs — Module autoload journal (PendingAutoload, Prepared→Replaced stages) for crash recovery (Phase 4)
    autoload_state.rs  — Derived autoload-state projection (NOT authoritative; lockfile is ground truth) (Phase 4)
    lifecycle_journal.rs — pending-lifecycle.json for update/remove/nupm_import crash recovery (Phase 5–6)
    nupm_import.rs     — nupm-import provenance (`state/nupm-imports.json`, Phase 6.2)
  nu/
    paths.rs           — Nu path cache (detect, load, save, validate_drift)
    autoload.rs        — render_use_statement, generate_autoload_content, FakeCandidateRunner, managed-file ops (Phase 4)
  util/
    atomic.rs          — write_json_atomic helper (tempfile+persist)
    fs_safety.rs       — OWNERSHIP_MARKER, acquire_mutation_lock (advisory fd_lock mutex), assert_managed_file_owned (Phase 4)
  nupm_compat/         — nupm discovery, import, drift (Phase 6.1–6.3); contract: docs/nupm-compatibility.md (compat-schema-v1)
    drift.rs           — compare_import, count_drifted_imports, DriftStatus (Phase 6.3)
    import.rs          — safe payload copy, lifecycle-journaled import transaction
    schema.rs          — COMPAT_SCHEMA_VERSION, parser caps, pinned nupm revision
    metadata.rs        — compat-schema-v1 metadata parser (ParsedMetadata, BehaviorFlags)
    classify.rs        — four-step classifier (NupmCompatibility)
    discovery.rs       — NupmHomeResolution, scan_nupm_home, inspect_path
    walk.rs            — bounded safe path walks (symlink_metadata)
    report.rs          — NupmStatusReport, NupmInspectionReport formatters
docs/
  nupm-compatibility.md — versioned nupm interoperability contract (authority for Phase 6)
tests/
  fixtures/nupm/       — supported/rejected fixture corpus for parser/classifier tests
  nupm_compat_test.rs  — Phase 6 integration tests (T13–T25, import/drift/manifest/activation/platform)
  nupm_real_nu_test.rs — Phase 6.4 real-Nu #[ignore] acceptance tests (run with `cargo test -- --ignored`)
```

## Key Conventions
- **Crate name**: `numan-cli`, **binary name**: `numan`
- **Product name**: Numan (capital N in prose, lowercase `numan` for CLI)
- **Edition**: Rust 2021
- **Error handling**: `anyhow` for application errors, `thiserror` for library errors
- **Serialization**: `serde` + `serde_json` (JSON) + `toml` (config)
- **CLI**: `clap` with derive macros
- **Platform detection**: `#[cfg(target_env)]` from binary's build target, not `std::env::consts`
- **Trust**: Ed25519 keys, `--key <base64-public-key>` for onboarding
- **Immutability**: `packages/<type>/<scoped-name>/<version-hash>/` paths, never overwrite
- **Activate testability**: `execute_with_registrar(args, root, registrar)` for plugins; `execute_with_candidate_runner(args, root, registrar, runner)` for modules — inject fakes in tests, never spawn a real Nu binary in unit tests
- **Module autoload testability**: `FakeCandidateRunner::success()` / `::failure(msg)` from `nu/autoload.rs` — use as test seam for candidate validation without real Nu
- **Module autoload identity**: Nu executable hash + Nu version + vendor autoload dir + managed file path — all four must match for a module to be considered active
- **Autoload state is NOT authoritative**: `autoload-state.json` is a fast-check projection; the lockfile `module_activation` records are ground truth
- **Managed file ownership**: `OWNERSHIP_MARKER` header identifies Numan-managed files; `assert_managed_file_owned` blocks overwrite of foreign files
- **Mutation serialization**: `acquire_mutation_lock(root)` returns `MutationLock` RAII guard; second acquire on same root fails immediately (non-blocking)
- **Nu invocation**: paths only via env vars (`NUMAN_PLUGIN_BINARY`, `NUMAN_PLUGIN_CONFIG`); the Nu program string is a compile-time constant with no runtime interpolation
- **Activation scope**: `PluginActivation` struct stores `(nu_executable_sha256, nu_version, plugin_registry_path)`; a plugin is "active" only when all three match the current `NuPaths` — bare `bool` would go stale after `numan init --refresh`
- **Journal**: `state/pending-activation.json` written as all-`prepared` before first registration; each entry advances to `registered` atomically before lockfile update; reconciled on next `activate` run if process is interrupted
- **Atomic writes**: all JSON state files (lockfile, journal, nu_state/paths.json) use `write_json_atomic` (tempfile in same dir + persist) — no partial-write corruption
- **Function signatures**: use `&Path` not `&PathBuf` in function parameters (clippy::ptr_arg is CI-enforced)

## Architecture Rules
1. **Install is always inert** — no Nu integration, only writes to `$NUMAN_ROOT`
2. **Activate is separate** — only command that touches Nu (plugin registration, autoloads)
3. **Source builds require consent** — prompt before clone/build, separate consent scope
4. **Lockfile pins immutable paths** — cached artifacts retained while referenced
5. **Registry trust** — Ed25519 signatures over exact index.json bytes

## Development Workflow
1. Create feature branch from `main`
2. Implement with tests
3. `cargo test` — all 234+ tests must pass
4. Update AGENTS.md if structure/conventions change
5. Open PR with description

## PR review guidance

Automated and human PR reviewers should follow [`.github/instructions/review.instructions.md`](.github/instructions/review.instructions.md) for review checklists, severity expectations, and architecture invariants to flag. Keep that file updated when review conventions change; link here rather than duplicating review rules in this doc.

## Dependencies
- clap (CLI), serde/serde_json/toml (serialization), reqwest (HTTP), tar/flate2/zip (archives)
- sha2/hex (integrity), ed25519-dalek/base64 (signatures), semver (versioning)
- dirs (platform paths), git2 (source builds), tempfile (safe extraction)

## Phase Status
- [x] Phase 1: Foundation (types, platform, config, lockfile, registry, trust, CLI skeleton)
- [x] Phase 2: Install transaction (download, verify, extract, lockfile write)
- [x] Phase 3: Activate command (plugin-only; `plugin add` via env-vars; journal recovery; drift detection)
- [x] Phase 4: Module autoload (render_use_statement, candidate validation, managed-file replacement, deactivation, journal recovery, mutation lock, 234+ tests)
- [x] Phase 5 (partial): Lockfile v2; `numan update/remove/gc`; pending-lifecycle journal
- [ ] Phase 5 (deferred): Source builds (5.2), lockfile snapshots/rollback (5.3), plugin gate (5.5)
- [x] Phase 6.0: nupm compatibility audit + fixture corpus (`docs/nupm-compatibility.md`)
- [x] Phase 6.1: read-only `numan nupm status|inspect` (no import, no nupm mutation, no Nu)
- [x] Phase 6.2: one-way `numan nupm import` (staging, provenance, lifecycle journal; no activation)
- [x] Phase 6.3: drift (`numan nupm diff`), status drift count, manifest import, re-import polish, activation tests
- [x] Phase 6.4: `--exit-on-ineligible`, parser fuzz, Unicode/symlink tests, real-Nu acceptance
- [ ] Phase 6 complete: publish compatibility matrix; CI acceptance job for `#[ignore]` real-Nu tests
- [ ] Phase 7: Polish, CI, distribution

## Testing
- Unit tests inline with source modules
- Integration tests in `tests/`
- Test-first approach: write test, verify failure, implement, verify pass
- All platform-specific code tested with mock platforms

## Error Patterns
- Use `anyhow::Result` for application code
- Use `thiserror` for library types that callers match on
- Include context with `.context("what failed")` or `?`
- Never panic in library code — return errors

## Git Conventions
- Commits: imperative mood, <72 chars
- Branches: `feature/description`, `fix/description`
- No force-push to main
- Squash merge for features
