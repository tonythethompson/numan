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

CI runs `cargo test`, `cargo clippy -- -D warnings`, and `cargo fmt --check` on Ubuntu, Windows, and macOS.

## PR review guidance

When reviewing pull requests (or preparing changes for review), follow [`.github/instructions/review.instructions.md`](.github/instructions/review.instructions.md). It defines CI gates, architecture invariants, mutation-lock expectations, and severity labels for findings.

## Architecture

Numan is a Rust CLI (`numan-cli` crate, `numan` binary) — a cross-platform package manager for Nushell.

**Module layout:**
- `src/core/` — pure domain logic: `package.rs` (types: `ScopedId`, `Package`, `VersionEntry`, `Artifact`), `platform.rs` (OS/arch detection), `registry.rs` (index load/search/verify), `trust.rs` (Ed25519 trust store), `integrity.rs` (SHA256), `resolve.rs` (semver resolution), `nu_version.rs` (Nu constraint matching)
- `src/cmd/` — thin clap subcommand handlers; each delegates to `core/` or `install/`
- `src/install/` — `download.rs` (HTTP), `extract.rs` (tar/zip), `transaction.rs` (full install flow: resolve → download → verify → extract → lockfile write)
- `src/state/lockfile.rs` — JSON lockfile with snapshot/rollback support
- `src/nu/paths.rs` — Nu path cache
- `src/config.rs` — root resolution (`--root` flag or platform default)

**Install path shape** (immutable): `<root>/packages/<type>/<owner>/<name>/<version>-<sha8prefix>/`

## Critical Rules

1. **Install is inert** — `install` writes only to `$NUMAN_ROOT`. It never touches Nu (no plugin registration, no autoload). The `activate` command is the only one that may touch Nu.
2. **Platform triple comes from `#[cfg(target_env)]`** at compile time, not `std::env::consts`. See `core/platform.rs` — `LIBC` is a compile-time const.
3. **Registry signatures are mandatory** — bypass requires `NUMAN_ALLOW_UNSIGNED=1` (dev only). Ed25519 signatures verified over exact `index.json` bytes.
4. **Artifact SHA256 is mandatory for plugins** — the install transaction bails if `sha256` is missing from a binary artifact.
5. **Lockfile snapshots before mutation** — `lockfile.snapshot()` called before any write if lockfile is non-empty.

## Error Handling

- `anyhow::Result` + `.context("what failed")` for application code
- `thiserror` for library types callers `match` on
- No panics in library code — return `Result`

## Phase Status

- Phase 1 (foundation): complete
- Phase 2 (install transaction): complete
- Phase 3 (activate plugins): complete — `cmd/activate.rs`, journal recovery, drift detection
- Phase 4 (activate modules/scripts/completions): complete — `nu/autoload.rs`, `state/autoload_state.rs`, `state/autoload_journal.rs`, `cmd/deactivate.rs`, `util/fs_safety.rs`
- Phase 5 (partial): complete — lockfile v2 (revision_id, payload_sha256, compute_revision_id), `numan update [--check]`, `numan remove [--force]`, `numan gc [--dry-run]`, pending-lifecycle.json journal; deferred: source builds, snapshots/rollback, plugin gate
- Phases 6–7: not yet started (nupm interop, polish)

See AGENTS.md for full conventions, git workflow, and dependency notes.
