# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- CI: MSRV job (1.85), `cargo deny`, `cargo package` checks; CI runs on version tags
- Release workflow gates on green CI + preflight (fmt, clippy, test, package) before build/publish
- Homebrew formula (`packaging/homebrew/numan.rb`) and winget manifests for v0.1.2
- [docs/PACKAGING.md](docs/PACKAGING.md) — packaging update checklist

### Changed

- README command reference: common flags table aligned with clap `--help`
- README: `cargo install --git`, Homebrew, and winget install paths
- [docs/RELEASING.md](docs/RELEASING.md): pre-tag local checklist and CI gate documentation

## [0.1.2] - 2026-06-30

### Added

- `numan doctor [--fix] [--yes] [--json]` — health checks and safe repairs ([docs/numan-doctor.md](docs/numan-doctor.md))
- `numan completions bash|fish|zsh|powershell` — shell completion scripts
- `util::hints` — canonical fix strings aligned with doctor output across init, install, activate, and nupm import
- First-init onboarding checklist after `numan init` (registry → sync → search → install → activate → doctor)

### Changed

- README quick start: single copy-paste onboarding path; doctor and completions documented
- Error messages in init, install, activate, and nupm import now include consistent `Run 'numan …'` fix hints

## [0.1.1] - 2026-06-30

### Added

- `numan init` and `numan init --refresh` for Nu path probing and activation identity refresh
- crates.io publishing (`cargo install numan-cli`) and [docs/RELEASING.md](docs/RELEASING.md)
- [CHANGELOG.md](CHANGELOG.md) and release checklist

## [0.1.0] - 2026-06-30

### Added

- Registry-backed install, activate, update, remove, and gc
- Module autoload with managed `numan.nu` vendor file
- nupm interoperability: status, inspect, import, diff
- GitHub Release binaries for Linux, Windows, and macOS
- Real-Nu acceptance CI job

[Unreleased]: https://github.com/tonythethompson/numan/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/tonythethompson/numan/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/tonythethompson/numan/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/tonythethompson/numan/releases/tag/v0.1.0
