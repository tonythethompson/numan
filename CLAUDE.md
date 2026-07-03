# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
cargo build                        # build
cargo run -- search <query>        # run
cargo test                         # all tests (245+)
cargo test core::resolve           # single module (replace with any module path)
cargo clippy -- -D warnings        # lint (CI enforces -D warnings)
cargo fmt                          # format
```

CI runs `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check`, and a real-Nu acceptance job (`cargo test -- --ignored` with Nu 0.113 on PATH) on Ubuntu, Windows, and macOS.

## PR review guidance

When reviewing pull requests (or preparing changes for review), follow [`.github/instructions/review.instructions.md`](.github/instructions/review.instructions.md). It defines CI gates, architecture invariants, mutation-lock expectations, and severity labels for findings.

## Architecture

Numan is a Rust CLI (`numan-cli` crate, `numan` binary) — a cross-platform package manager for Nushell.

**Module layout:**
- `src/core/` — pure domain logic: `package.rs` (types: `ScopedId`, `Package`, `VersionEntry`, `Artifact`), `platform.rs` (OS/arch detection), `registry.rs` (index load/search/verify), `trust.rs` (Ed25519 trust store), `integrity.rs` (SHA256), `resolve.rs` (semver resolution), `nu_version.rs` (Nu constraint matching)
- `src/cmd/` — thin clap subcommand handlers; each delegates to `core/` or `install/`
- `src/install/` — `download.rs` (HTTP), `extract.rs` (tar/zip), `transaction.rs` (full install flow: resolve → download → verify → extract → lockfile write)
- `src/state/lockfile.rs` — JSON lockfile (authoritative install/activation state)
- `src/state/snapshot.rs` — immutable activation snapshots (`create_snapshot`, `list_snapshots`, `delete_snapshot`)
- `src/state/rollback.rs` — journaled restore of Numan-owned state to a snapshot
- `src/nu/paths.rs` — Nu path cache
- `src/config.rs` — root resolution (`--root` flag or platform default)

**Install path shape** (immutable): `<root>/packages/<type>/<owner>/<name>/<version>-<sha8prefix>/`

## Critical Rules

1. **Install is inert** — `install` writes only to `$NUMAN_ROOT`. It never touches Nu (no plugin registration, no autoload). The `activate` command is the only one that may touch Nu.
2. **Platform triple comes from `#[cfg(target_env)]`** at compile time, not `std::env::consts`. See `core/platform.rs` — `LIBC` is a compile-time const.
3. **Registry signatures are mandatory** — bypass requires `NUMAN_ALLOW_UNSIGNED=1` (dev only). Ed25519 signatures verified over exact `index.json` bytes.
4. **Artifact SHA256 is mandatory for plugins** — the install transaction bails if `sha256` is missing from a binary artifact.
5. **State snapshots before mutation** — `create_snapshot()` called before `install`/`update`/`remove`/`activate`/`deactivate`/nupm-import mutations. `numan gc` treats every snapshot's referenced payloads as live roots.

## Error Handling

- `anyhow::Result` + `.context("what failed")` for application code
- `thiserror` for library types callers `match` on
- No panics in library code — return `Result`

## Phase Status

- Phase 1 (foundation): complete
- Phase 2 (install transaction): complete
- Phase 3 (activate plugins): complete — `cmd/activate.rs`, journal recovery, drift detection
- Phase 4 (activate modules/scripts/completions): complete — `nu/autoload.rs`, `state/autoload_state.rs`, `state/autoload_journal.rs`, `cmd/deactivate.rs`, `util/fs_safety.rs`
- Phase 5.1/5.4: complete — lockfile v2 (revision_id, payload_sha256, compute_revision_id), `numan update [--check]`, `numan remove [--force]`, `numan gc [--dry-run]`, pending-lifecycle.json journal
- Phase 5.3 (immutable activation snapshots and rollback): complete — `state/snapshot.rs`, `state/rollback.rs`, `numan snapshot list|inspect|delete|rollback`; see [docs/snapshots-and-rollback.md](docs/snapshots-and-rollback.md). Deferred: source builds (5.2), plugin lifecycle gate (5.5, issue #22)
- Phases 6–7: complete (nupm interop, distribution/polish) — see `Phase6Plan.md`, `Phase7Plan.md`

See AGENTS.md for full conventions, git workflow, and dependency notes.
