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

# Test (all 38 tests)
cargo test

# Test single module
cargo test core::platform
cargo test core::package
cargo test core::resolve
cargo test state::lockfile
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
  install/
    download.rs        — HTTP download with progress
  state/
    lockfile.rs        — Lockfile with registry fields, snapshot support
  nu/
    paths.rs           — Nu path cache (detect, load, save)
  nupm_compat/         — nupm interoperability adapter (future)
tests/                 — Integration tests
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

## Architecture Rules
1. **Install is always inert** — no Nu integration, only writes to `$NUMAN_ROOT`
2. **Activate is separate** — only command that touches Nu (plugin registration, autoloads)
3. **Source builds require consent** — prompt before clone/build, separate consent scope
4. **Lockfile pins immutable paths** — cached artifacts retained while referenced
5. **Registry trust** — Ed25519 signatures over exact index.json bytes

## Development Workflow
1. Create feature branch from `main`
2. Implement with tests
3. `cargo test` — all 38+ tests must pass
4. Update AGENTS.md if structure/conventions change
5. Open PR with description

## Dependencies
- clap (CLI), serde/serde_json/toml (serialization), reqwest (HTTP), tar/flate2/zip (archives)
- sha2/hex (integrity), ed25519-dalek/base64 (signatures), semver (versioning)
- dirs (platform paths), git2 (source builds), tempfile (safe extraction)

## Phase Status
- [x] Phase 1: Foundation (types, platform, config, lockfile, registry, trust, CLI skeleton)
- [ ] Phase 2: Install transaction (download, verify, extract, lockfile write)
- [ ] Phase 3: Activate command (plugin registration, autoload generation)
- [ ] Phase 4: Module/script/completion install
- [ ] Phase 5: Source builds, update, remove
- [ ] Phase 6: nupm interop
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
