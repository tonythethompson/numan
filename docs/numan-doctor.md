# `numan doctor` specification

**Status:** Planned (Phase 7, slice 2)  
**Authority:** This document defines behavior for `numan doctor` before implementation.

## Purpose

`numan doctor` is a **read-only** health command that aggregates checks currently spread across `init`, `activate --check`, journal recovery, and `nupm status`. It answers: *“Is this Numan root consistent, safe to mutate, and aligned with the current Nu environment?”*

It **reports** problems and prints **fix hints**. It does **not** reconcile journals, activate packages, modify nupm, or write state.

## Non-goals

- No mutation of lockfile, journals, managed files, or nupm trees
- No substitute for `numan activate` (registration / autoload writes)
- No registry sync or package install
- No automatic `init --refresh` (only recommends it)
- No execution of `build.nu` or nupm import

## Invocation

```text
numan doctor [--json] [--nupm-home PATH]
```

| Flag | Behavior |
|------|----------|
| `--json` | Emit a single JSON object (schema versioned); no ANSI styling |
| `--nupm-home PATH` | Override nupm home for the optional coexistence section (same resolution order as `numan nupm status`) |

Global `--root` applies as for all commands.

## Exit codes

| Code | Meaning |
|------|---------|
| `0` | No **error**-severity findings (warnings and info allowed) |
| `1` | One or more **error**-severity findings |
| `2` | Cannot run meaningful checks (e.g. not initialized, unreadable root) |

## Severity model

Each finding has:

- `id` — stable machine identifier (e.g. `nu_paths.missing`)
- `severity` — `ok` \| `info` \| `warn` \| `error`
- `message` — human-readable summary
- `fix` — optional suggested command (e.g. `numan init --refresh`)

**Rules:**

- `error` — blocks safe mutation until resolved (drift, stale journal with wrong identity, missing payload for active package)
- `warn` — operational risk or incomplete setup (no registries, nupm drift, pending journal from interrupted run)
- `info` — contextual only (nupm home not configured, no packages installed)
- `ok` — check passed (included in `--json`; omitted in default human output unless `--verbose` is added later)

## Check catalog

Checks run in order below. Implementation should call existing validators (`NuPaths::validate_drift`, `AutoloadState::validate_against_lockfile`, etc.) rather than duplicating logic.

### 1. Root layout

| ID | Severity if failed | Condition |
|----|-------------------|-----------|
| `root.writable` | `error` | Numan root exists and is writable |
| `layout.nu_state` | `warn` | `nu_state/` present |
| `layout.state` | `warn` | `state/` present |

### 2. Initialization (`nu_state/paths.json`)

| ID | Severity | Condition |
|----|----------|-----------|
| `nu_paths.missing` | `error` | `paths.json` absent → fix: `numan init` |
| `nu_paths.drift` | `error` | `NuPaths::validate_drift()` fails → fix: `numan init --refresh` |
| `nu_paths.vendor_drift` | `error` | `validate_vendor_drift()` fails when `data_dir` cached → fix: `numan init --refresh` |
| `nu_paths.vendor_missing` | `warn` | Active module in lockfile but `vendor_autoload_dir` is `None` → fix: fix Nu install/config, then `numan init --refresh` |

### 3. Pending journals (report only — do not reconcile)

Per `state/journal.rs`: *“`numan doctor` reports it without acting.”*

| ID | Severity | Condition |
|----|----------|-----------|
| `journal.plugin_pending` | `warn` | `state/pending-activation.json` exists; include stage counts |
| `journal.plugin_stale` | `error` | Journal Nu identity ≠ current `NuPaths` → fix: `numan init --refresh` then `numan activate` |
| `journal.autoload_pending` | `warn` | `state/pending-autoload.json` exists; include `stage` |
| `journal.autoload_stale` | `error` | Journal identity mismatch → fix: `numan init --refresh` |
| `journal.lifecycle_pending` | `warn` | `state/pending-lifecycle.json` exists; include `op` + `stage` |
| `journal.lifecycle_stale` | `error` | Stale lifecycle journal (reuse `check_stale_journal` semantics) → fix: per op docs / manual recovery |

### 4. Lockfile and activation identity

| ID | Severity | Condition |
|----|----------|-----------|
| `lockfile.missing` | `info` | No lockfile or empty → nothing installed |
| `lockfile.parse` | `error` | Lockfile unreadable or invalid JSON |
| `activation.plugin_stale` | `warn` | Plugin has `activation` but `is_active_for` false for current `NuPaths` |
| `activation.module_stale` | `warn` | Module has `module_activation` but `is_module_active_for` false |
| `autoload.projection` | `error` | `AutoloadState::validate_against_lockfile` fails |
| `autoload.managed_missing` | `warn` | Active modules but managed `numan.nu` absent |
| `autoload.managed_foreign` | `error` | Managed file exists but fails `assert_managed_file_owned` |

Reuse the same checks as `numan activate --check` where applicable, but run for **all** active modules/plugins without requiring `--check` on activate.

### 5. Payload presence (lightweight)

| ID | Severity | Condition |
|----|----------|-----------|
| `payload.missing` | `error` | Lockfile entry references `payload_path` that does not exist under root |

No re-hash or revision recompute in v1 (too expensive for doctor).

### 6. Registry configuration

| ID | Severity | Condition |
|----|----------|-----------|
| `registry.none` | `warn` | `config.toml` has no registries → fix: `numan registry add …` |
| `registry.index_missing` | `info` | Enabled registry has no cached index under `registries/` → fix: `numan registry sync` |

### 7. nupm coexistence (optional section)

Controlled by `config.toml` → `[nupm_compat] scan_on_doctor` (default `true`). When `false`, skip section entirely.

When enabled:

- If `NUPM_HOME` / `--nupm-home` unavailable: `info` finding `nupm.home_unconfigured` (not an error)
- Else: run read-only discovery (same as `numan nupm status` classification counts)
  - `nupm.drift` — `warn` if `source_drift_count > 0` → fix: `numan nupm diff <pkg>`
  - `nupm.overlap` — `info` if `name_overlap_count > 0`

Never write under nupm home.

## Human output format

```text
Numan doctor — <root>

Initialization
  ✓ Nu paths cached (0.113.1)
  ✓ Nu binary hash matches

Journals
  ⚠ Pending lifecycle journal (op: nupm_import, stage: StagingPayload)
    Fix: complete or clear per docs/RELEASING.md …

Activation
  ✓ Plugin owner/foo active for current Nu
  ✗ Autoload-state projection mismatch: …
    Fix: numan activate owner/module-name

nupm coexistence
  · nupm home not configured (pass --nupm-home or set NUPM_HOME)

Summary: 1 error, 1 warning
```

Use `console` styling consistent with `activate --check`.

## JSON output format (v1)

```json
{
  "schema_version": 1,
  "root": "/path/to/numan",
  "summary": { "errors": 1, "warnings": 1, "infos": 0 },
  "findings": [
    {
      "id": "autoload.projection",
      "severity": "error",
      "message": "…",
      "fix": "numan activate …"
    }
  ]
}
```

## Architecture

| Piece | Location |
|-------|----------|
| CLI | `src/cmd/doctor.rs` |
| Dispatch | `src/main.rs` → `Commands::Doctor` |
| Config gate | `config::NupmCompatConfig::scan_on_doctor` |
| Tests | `tests/doctor_test.rs` + inline unit tests; **no real Nu** |

Public test seam (if needed):

```rust
pub fn execute_with_options(args: &DoctorArgs, root: &Path, options: DoctorOptions) -> Result<DoctorReport>
```

## Relationship to existing commands

| Command | Role |
|---------|------|
| `numan init` / `init --refresh` | **Fix** Nu path drift (doctor only suggests) |
| `numan activate` | **Fix** activation; reconciles plugin/autoload journals |
| `numan activate --check` | Deep **module** check; doctor subsumes into broader report |
| `numan nupm status` | nupm-only summary; doctor embeds optional subset |
| `numan update` / `remove` / `gc` | Block on stale lifecycle journal; doctor **reports** it |

## Definition of done

- [ ] `numan doctor` and `numan doctor --json` implemented per check catalog
- [ ] `scan_on_doctor` respected
- [ ] No state mutation; verified by test that root mtime / file hashes unchanged
- [ ] Documented in README command table and `AGENTS.md`
- [ ] Integration tests for: uninitialized root, drifted paths, pending journals, projection mismatch

## Changelog

| Date | Change |
|------|--------|
| 2026-06-30 | Initial spec (Phase 7.2) |
