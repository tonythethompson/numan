# Numan

[![CI](https://github.com/tonythethompson/numan/actions/workflows/ci.yml/badge.svg)](https://github.com/tonythethompson/numan/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**Numan** is a cross-platform package manager for [Nushell](https://www.nushell.sh/). It installs plugins, modules, scripts, and completions from signed registries, pins immutable artifacts in a lockfile, and activates them with Nu only when you ask — keeping installs inert until you run `numan activate`.

Built in Rust for Linux, macOS, and Windows.

---

## About

Nushell’s ecosystem has grown around community packages, but managing them across machines, Nu versions, and platforms is still painful. **nupm** covers local installs well; registry-based workflows need verified artifacts, reproducible lockfiles, and safe activation that survives Nu upgrades.

Numan fills that gap:

| Concern | How Numan handles it |
|--------|----------------------|
| **Trust** | Ed25519 signatures over registry indexes; SHA256 verification of plugin binaries |
| **Reproducibility** | Lockfile v2 pins version, payload hash, and install origin |
| **Platform safety** | Artifacts resolved for the compile-time OS/arch/libc triple |
| **Nu version matching** | Resolver respects per-package Nu constraints |
| **Activation isolation** | `install` never touches Nu; only `activate` registers plugins or writes autoloads |
| **Crash recovery** | Journals for activation, autoload, lifecycle, and nupm import operations |
| **nupm coexistence** | Read-only discovery, one-way import, and drift detection for existing nupm installs |

Numan is **early-stage** (v0.1.0). Core install, activate, update, remove, gc, registry, and nupm interoperability are implemented and covered by 245+ tests plus real-Nu acceptance on CI. Source builds, lockfile rollback snapshots, and distribution packaging are planned for later phases.

---

## Features

- **Registry-backed installs** — search, inspect versions, and install `owner/name` or `owner/name@version`
- **Package types** — plugins, modules, scripts, completions
- **Verified artifacts** — mandatory SHA256 for plugin binaries; signed registry indexes
- **Scoped activation** — plugins active only when Nu executable hash, Nu version, and plugin registry path match
- **Module autoloads** — managed vendor autoload files with ownership markers and candidate validation
- **Lifecycle management** — `update`, `remove`, and `gc` with pending-lifecycle journal recovery
- **nupm interoperability** — `numan nupm status|inspect|import|diff` for migration from [nupm](https://github.com/nushell/nupm)
- **Cross-platform** — tested on Ubuntu, Windows, and macOS in CI

---

## Installation

### From source

Requires [Rust](https://rustup.rs/) (stable) and a Nushell binary on `PATH` for activation commands.

```bash
git clone https://github.com/tonythethompson/numan.git
cd numan
cargo install --path .
```

The binary is named `numan`.

### Pre-built releases

Release binaries are not published yet. Track [Releases](https://github.com/tonythethompson/numan/releases) for updates.

---

## Quick start

### 1. Configure a registry

Add a registry with its Ed25519 public key, then sync the index:

```bash
numan registry add official https://example.com/index.json --key <base64-public-key>
numan registry sync
```

### 2. Search and install

```bash
numan search hooks
numan info owner/package-name
numan install owner/package-name
numan list
```

Install is **inert** — nothing is registered with Nu until you activate.

### 3. Activate with Nu

```bash
numan activate                    # activate all inactive packages
numan activate owner/package-name # activate specific packages
numan activate --list             # show activation status
numan activate --check            # verify activation integrity (read-only)
```

For modules:

```bash
numan deactivate owner/module-name
```

### 4. Maintain installs

```bash
numan update --check              # see available upgrades
numan update                      # apply upgrades
numan remove owner/package-name
numan gc --dry-run                # preview orphaned payload dirs
numan gc                          # delete unreferenced payloads
```

---

## Data layout

By default, Numan stores state under a platform-specific root (override with `NUMAN_ROOT` or `--root`):

| Platform | Default root |
|----------|--------------|
| Linux | `~/.local/share/numan` |
| macOS | `~/Library/Application Support/numan` |
| Windows | `%LOCALAPPDATA%\numan` |

Important paths under the root:

```text
numan/
├── config.toml          # registries, defaults
├── lockfile.json        # pinned installs (authoritative)
├── packages/            # immutable versioned payloads
├── registries/          # synced index caches
├── state/               # journals, nupm import provenance
└── nu_state/            # cached Nu paths for activation checks
```

Payload paths are immutable: `packages/<type>/<owner>/<name>/<version>-<hash>/`.

---

## Command reference

| Command | Description |
|---------|-------------|
| `numan search <query>` | Search registry by name, description, or tags |
| `numan info <owner/name>` | Show package metadata and available versions |
| `numan install <owner/name[@version]>` | Download, verify, extract, and lock |
| `numan list` | List installed packages and activation status |
| `numan activate [pkg...]` | Register plugins / write module autoloads |
| `numan deactivate [pkg...]` | Remove module autoload entries |
| `numan update [--check] [pkg]` | Upgrade installed packages |
| `numan remove [--force] <pkg>` | Remove from lockfile and delete payload |
| `numan gc [--dry-run]` | Delete orphaned package directories |
| `numan registry list\|sync\|add\|remove\|packages` | Registry management |
| `numan nupm status` | Summarize nupm home and import eligibility |
| `numan nupm inspect [--all] [path]` | Classify nupm packages at a path |
| `numan nupm import [--as owner/name] [path]` | One-way import into Numan |
| `numan nupm import --manifest file.toml` | Batch import from manifest |
| `numan nupm diff <owner/name>` | Compare imported payload vs nupm source |

Global flag: `--root <path>` — override the Numan root directory.

Run `numan <command> --help` for full flag documentation.

---

## nupm migration

Numan can discover and import compatible packages from an existing [nupm](https://github.com/nushell/nupm) installation without modifying nupm state.

```bash
# Point at nupm home (or rely on $NUPM_HOME)
numan nupm status --nupm-home ~/.config/nupm
numan nupm inspect --all --nupm-home ~/.config/nupm

# Import a supported module package
numan nupm import /path/to/package --as myorg/my-module --yes

# Check drift after the source changes
numan nupm diff myorg/my-module
```

Supported and rejected package shapes are documented in [docs/nupm-compatibility.md](docs/nupm-compatibility.md).

---

## Design principles

1. **Install is inert** — installs write only to `$NUMAN_ROOT`; Nu is never invoked.
2. **Activate is explicit** — the only command that registers plugins or manages autoload files.
3. **Lockfile is ground truth** — derived state (e.g. autoload projections) is not authoritative.
4. **Immutable payloads** — versions are content-addressed; updates leave old dirs until `gc`.
5. **Mutation serialization** — advisory locks prevent concurrent destructive operations.
6. **Safe Nu invocation** — plugin paths are passed via environment variables, not interpolated into shell strings.

See [AGENTS.md](AGENTS.md) for architecture details aimed at contributors and agents.

---

## Development

```bash
cargo build
cargo test                    # unit + integration (245+ tests)
cargo clippy -- -D warnings   # lint (CI-enforced)
cargo fmt                     # format

# Real-Nu acceptance tests (requires Nu 0.113+ on PATH)
cargo test -- --ignored
```

CI runs tests, clippy, `rustfmt --check`, and real-Nu acceptance on Ubuntu, Windows, and macOS.

### Contributing

1. Branch from `main` (`feature/...` or `fix/...`).
2. Add or update tests for behavior changes.
3. Ensure `cargo test` and `cargo clippy -- -D warnings` pass.
4. Open a pull request with a clear description and test plan.

PR reviewers should follow [`.github/instructions/review.instructions.md`](.github/instructions/review.instructions.md).

---

## Roadmap

| Phase | Status |
|-------|--------|
| Foundation, install, activate (plugins + modules) | ✅ Complete |
| Update, remove, gc, lockfile v2 | ✅ Complete |
| nupm status, inspect, import, drift | ✅ Complete |
| Source builds, lockfile rollback snapshots | 🔜 Planned |
| Polish, CI hardening, distribution | 🔜 Planned |

---

## License

MIT — see [LICENSE](LICENSE).

---

## Related projects

- [Nushell](https://www.nushell.sh/) — the shell Numan packages for
- [nupm](https://github.com/nushell/nupm) — Nushell’s built-in package manager; Numan interoperates via import and drift detection
