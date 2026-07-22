# Official registry Stage 1 acceptance

The Stage 1 acceptance harness is a manual, production-registry integration test for the Windows x64 Numan release path. It spawns the built `numan` executable directly and exercises this lifecycle:

```text
init → registry sync → search → info → install → activate → doctor → list → deactivate → remove → gc
```

Stage 1 deactivates the plugin (clears the activation record, keeps the payload), then removes and garbage-collects. Active-plugin `remove` while activated (including `--force`) remains refused; deactivate first. See [docs/active-plugin-gate.md](../active-plugin-gate.md).

The default subject is the official-registry package `fdncred/nu_plugin_file`, queried as `nu_plugin_file`. The production resolver chooses the compatible version for Windows x64 and Nu 0.113.x; the harness does not duplicate version-selection logic or hard-code a package version.

## Scope and non-goals

Stage 1 proves that a clean Numan root can consume the signed production official registry, install and activate a compatible plugin, and report healthy state through `doctor` and `list`. It also records the signing key ID, index digest, lockfile provenance, executable digest, Nu plugin-registry changes, lifecycle journals, and snapshots.

The harness does not change CLI behavior, registry trust policy, unsigned-registry fallback, retry failed commands, install Nushell, test non-Windows targets, test other Nu minor versions, or call production command handlers in-process. It is not a replacement for ordinary unit and integration tests.

## Prerequisites

- Windows x86_64.
- Nushell 0.113.x available as `nu` on `PATH`. The manual workflow pins 0.113.1 exactly.
- Network access to the compiled production official-registry URL and the selected package artifact.
- A built test binary produced by Cargo.

An operating-system, architecture, or Nu-version mismatch is recorded as a failed preflight, not skipped. Do not run the live test with another Nu minor version. In particular, a local Nu 0.114.x installation is not an authoritative Stage 1 environment.

## Invocation

Run the ordinary, hermetic infrastructure coverage with:

```powershell
cargo test --locked --test official_registry_stage1
```

Run the live lifecycle only when `nu --version` reports 0.113.x:

```powershell
cargo test --locked --test official_registry_stage1 stage1_official_registry -- --ignored --nocapture --test-threads=1
```

The repository's **Official Registry Stage 1 Acceptance** workflow performs the authoritative manual run with Nu 0.113.1. It is `workflow_dispatch` only, requires no secrets, and uploads evidence even when the test fails.

## Overrides

The following optional variables change the test subject or evidence parent directory:

| Variable | Default | Meaning |
|---|---|---|
| `NUMAN_ACCEPTANCE_PACKAGE` | `fdncred/nu_plugin_file` | Exact `owner/name` package ID. |
| `NUMAN_ACCEPTANCE_QUERY` | `nu_plugin_file` | Search query that must return an exact package-ID row. |
| `NUMAN_ACCEPTANCE_OUTPUT` | `target/acceptance/official-registry-stage1` | Parent directory beneath which a unique run directory is created. |

Package overrides must still resolve to an activatable plugin with a Windows x64 artifact compatible with Nu 0.113.x. The override does not weaken any lifecycle assertion.

## Isolation guarantees

Every run creates a unique `<utc-unix-ms>-<uuid-prefix>` directory. It creates `home/` and `evidence/` first and deliberately leaves `root/` absent until the first Numan invocation.

Child processes start from `env_clear()`. The harness preserves only `PATH` and the Windows process variables required to launch executables, sets `HOME`, `USERPROFILE`, `APPDATA`, `LOCALAPPDATA`, the three `XDG_*` homes, `TEMP`, and `TMP` beneath the isolated home, and sets `NO_COLOR=1`. It does not pass `NUMAN_ALLOW_UNSIGNED`, `NUMAN_ROOT`, `NUPM_HOME`, tokens, or the complete parent environment.

After `init`, every mutable path reported in `nu_state/paths.json`—the plugin registry, Nu data directory, every vendor-autoload directory, and the selected vendor-autoload target—must remain under the isolated home. The Nu executable itself may be outside the run directory. A path escape stops the lifecycle before activation.

## Evidence layout

The default layout is:

```text
target/acceptance/official-registry-stage1/
└── <utc-ms>-<uuid-prefix>/
    ├── home/
    ├── root/
    └── evidence/
        ├── run.json
        ├── summary.json
        ├── summary.md
        ├── 00-preflight/
        ├── 01-init/
        ├── 02-registry-sync/
        └── ...
```

Every command directory contains:

- `command.json`: program, arguments, timeout, UTC millisecond timestamps, duration, exit status, timeout state, byte counts, and stdout/stderr SHA-256 digests.
- `stdout.txt` and `stderr.txt`: exact captured streams.
- `root-files.json`: sorted, normalized root inventory with type, size, symlink target, and streaming file digest. Directory symlinks are never followed.
- `state.json`: selected configuration and state files, known and discovered journals, snapshots, registry index/signature/derived-key/last-known-good files, and the isolated Nu plugin registry.

JSON, TOML, and text state up to 1 MiB is inlined. Larger or binary files retain metadata and a streaming digest. Inventory hashes are reused by state evidence instead of reading root payloads twice. `run.json`, `summary.json`, and `summary.md` are finalized on both success and assertion failure.

The workflow uploads only `evidence/`; it does not upload the isolated home or installed payload tree.

## Failure diagnosis

The test writes current-step evidence before evaluating assertions. On failure, later lifecycle steps do not run. The final panic reports the failed step, arguments, exit code or timeout, all current-step assertion errors, stdout/stderr paths, and the evidence directory.

Start with `summary.md`, then inspect that step's `command.json`, streams, inventory, and state capture. A timeout means the process was killed and reaped; Stage 1 never retries. Registry-sync evidence distinguishes a fresh signed index from cached or last-known-good fallback. Doctor uses its JSON schema and accepts no warning IDs in Stage 1 (info findings such as `activation.plugin_mutation_gated` are allowed).

## Remaining payloads after Stage 1

Because Stage 1 ends with an activated plugin still present, the final summary may list package directories that are referenced by the current lockfile (and possibly snapshots). Those are not orphans. Full remove → gc payload accounting returns when plugin deactivation clears the activation record and Stage 1 regain remove/gc steps.

The live test is ignored by default because it depends on production network state, an exact platform/Nu compatibility window, and activation of a real plugin. Keeping it manual prevents ordinary `cargo test` runs from silently becoming networked or host-mutating tests while retaining an explicit, reproducible release gate.
