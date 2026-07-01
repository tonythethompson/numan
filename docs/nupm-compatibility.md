# nupm compatibility contract

**compat-schema-version:** `1`  
**migration-schema-version:** `1`

This document is the **versioned compatibility contract** for Numan's nupm interoperability. Phase 6.1+ implementation must derive metadata grammar, classifier rules, discovery bounds, and test expectations (T01–T15) exclusively from this file and [tests/fixtures/nupm/](tests/fixtures/nupm/).

**Authority:** This document + fixture corpus. [Phase6Plan.md](../Phase6Plan.md) is planning reference only.

**Phase 6.1 ships:** read-only `numan nupm status` and `numan nupm inspect`. Phase 6.2 adds `numan nupm import`. Phase 6.3 adds drift detection, bulk manifest import, and activation verification. Phase 6.5 adds the migration planner with stable outcomes, reason codes, and `--json` reporting.

---

## Pinned nupm revision

| Field | Value |
|-------|-------|
| Repository | [nushell/nupm](https://github.com/nushell/nupm) |
| Commit | `421eee1c5ec9a8d751c4480157dcfcabf9d7b963` |
| Date | 2026-01-24 |
| Note | Latest `main` at audit time; includes `$nu.temp-dir` rename fix (#127) |
| Pin file | `tests/fixtures/nupm/pinned-nupm-revision.txt` |

### Sources reviewed at this revision

| Path | Purpose |
|------|---------|
| `nupm/utils/dirs.nu` | `NUPM_HOME`, `modules/`, `scripts/`, `PACKAGE_FILENAME` |
| `nupm/utils/package.nu` | Required metadata keys (`name`, `version`, `type`) |
| `nupm/install.nu` | Type dispatch: `module`, `script`, `custom` |
| `nupm.nuon` | Real module metadata (optional `description`, `license`) |
| `tests/packages/spam_*` | Official test corpus for all three package types |
| `registry/*.nuon` | Registry index entries (mostly `git`-sourced packages) |

### External reference (not pinned)

Registry packages such as `nu-hooks` install from git (`nushell/nu_scripts`). Their on-disk layout after `nupm install` matches the **module** rules below: package root contains `nupm.nuon` and a subdirectory named after `$.name` with `mod.nu`.

---

## nupm layout and discovery

### NUPM_HOME resolution (nupm itself)

nupm requires `$env.NUPM_HOME` to be set before most commands run (`nupm-home-prompt` in `dirs.nu`). If unset, nupm errors internally.

nupm's **default** when bootstrapping (not auto-used by Numan):

| Platform | Typical default |
|----------|-----------------|
| Linux/macOS | `$nu.default-config-dir/nupm` (often `~/.config/nupm` or XDG equivalent) |
| Windows | `%APPDATA%\nushell\config\nupm` or similar under default config dir |

Users commonly override with `$env.NUPM_HOME = ($env.XDG_DATA_HOME \| path join "nupm")`.

### Directory structure under NUPM_HOME

| Subdirectory | Role |
|--------------|------|
| `modules/` | Installed **module** packages (`module-dir`) |
| `scripts/` | Installed **script** binaries and module auxiliary scripts |
| `cache/` | Download cache (`NUPM_CACHE`, default under config dir) |
| `overlays/` | Documented in design docs; not present in pinned install code paths |

### Package root discovery

nupm locates a package root by walking parents from a path until `nupm.nuon` is found (`find-root` in `dirs.nu`). Numan `inspect <path>` uses the same walk for **source package roots**.

Under `$NUPM_HOME/modules/<name>/`, nupm stores only the installed module tree (no `nupm.nuon`). Phase 6.1 `inspect --all` must enumerate those directories separately and report missing metadata when no sibling source root is available.

### Installed module layout (nupm `install-path` for `type: module`)

After `nupm install --path <pkg_dir>`:

```text
$NUPM_HOME/modules/<name>/          ← copy of <pkg_dir>/<name>/ only
<pkg_dir>/nupm.nuon                 ← metadata stays at source unless copied
```

Source package directory (before install):

```text
<pkg_dir>/
  nupm.nuon
  <name>/          ← required subdirectory matching $.name
    mod.nu         ← module entry (nupm convention)
  script.nu        ← optional; installed to scripts/ if $.scripts present
```

**Module entry convention:** `<pkg_dir>/<declared_name>/mod.nu`.

nupm validates that `<pkg_dir>/<name>` is a directory; it does not require `mod.nu` at install time, but module activation uses `use <name>` which expects a loadable module.

---

## `nupm.nuon` metadata shapes observed

### Required fields (nupm `open-package-file`)

| Field | Type in fixtures | Notes |
|-------|------------------|-------|
| `name` | bare identifier or string | e.g. `spam_module` or `"nu-hooks"` |
| `version` | string | Semver-like string; nupm does not parse semver strictly in metadata |
| `type` | bare identifier | One of `module`, `script`, `custom` |

### Optional fields observed in pinned nupm sources

| Field | Type | Seen in |
|-------|------|---------|
| `description` | string | Root `nupm.nuon` |
| `license` | string or bare word | Root `nupm.nuon` (`LICENSE`) |
| `scripts` | list of strings | `tests/packages/spam_module`, `spam_script` |

### Fields not observed in pinned nupm `nupm.nuon` files

These appear in design discussions or Phase 6 planning but **were not found** in the five `nupm.nuon` files at the pinned commit. Fixtures still cover them as **rejected** cases:

| Field | Treatment |
|-------|-----------|
| `deps` / `dependencies` | Reject — external dependency metadata |
| `build` / hooks | Reject — use `build.nu` file presence instead |
| Closures, `$variables`, dates | Reject — outside NUON subset |

### NUON syntax rules (for Phase 6.1 parser)

Observed in real nupm files:

- Record literals use `{` `}` with **optional commas** between fields.
- Bare identifiers are valid string values (`name: spam_module`).
- Double-quoted strings for version and human text.
- Single-quoted strings in lists (`['spam_bar.nu']`).
- Bare identifiers in string lists (`[script.nu]`) — matches pinned nupm `spam_module`.

Numan's metadata parser (Phase 6.1) implements **compat-schema-v1** only. Diagnostics include `compat-schema-v1` in error messages. Reject closures, `$variables`, dates, binary literals, and unbounded nesting anywhere.

### Parser output model

The parser produces `ParsedMetadata` with `BehaviorFlags`:

```rust
struct ParsedMetadata {
    name: String,
    version: String,
    package_type: String,
    description: Option<String>,
    license: Option<String>,
    behavior: BehaviorFlags,
}
struct BehaviorFlags {
    has_scripts: bool,
    has_dependencies: bool,
}
```

- Parser **recognizes** `scripts`, `deps`, `dependencies`, `requires`; sets flags; does not retain values.
- **Unknown top-level fields**, **duplicate keys**, and **malformed known-field values** → `InvalidMetadata`.
- Classifier maps flags to `DeferredScript` / `UnsupportedDependencies` (not the parser).

### Field-specific grammar (compat-schema-v1)

Top-level record only; **trailing content after the closing `}` → reject.** **Duplicate keys → reject.**

| Field | Accepted shape | Notes |
|-------|----------------|-------|
| `name` | quoted string or bare identifier | One safe path component |
| `version` | quoted string only | No semver validation in 6.1 |
| `type` | quoted string or bare identifier | `module` / `script` / `custom` |
| `description` | quoted string only | Optional |
| `license` | quoted string or bare identifier | Optional |
| `scripts` | bounded list of scalar strings/identifiers | Sets `has_scripts` |
| `deps` / `dependencies` / `requires` | bounded record or list | Sets `has_dependencies` |
| *(other)* | — | `InvalidMetadata` |

### Parser caps

```text
MAX_METADATA_BYTES = 65536
MAX_TOKEN_COUNT = 4096
MAX_NESTING_DEPTH = 2
MAX_RECORD_FIELDS = 16
MAX_LIST_LENGTH = 64
MAX_STRING_LEN = 4096
```

`read_metadata_limited(path)` reads at most `MAX_METADATA_BYTES + 1` bytes before parsing.

### Classifier pipeline (four steps)

```text
Step 1 — Pre-parse path-chain safety → UnsafeFilesystemLayout
Step 2 — Parse metadata → InvalidMetadata
Step 3 — Metadata-dependent layout (parsed.name) → UnsafeFilesystemLayout
Step 4 — Precedence: UnsupportedCustomBuild → UnsupportedDependencies →
         DeferredScript → UnknownType → ImportableModule
```

`build.nu` is detected in Step 3 but classified as `UnsupportedCustomBuild` in Step 4.

### Migration outcomes (Phase 6.5)

Every source root is assessed into a stable `NupmOutcome` with one or more `NupmReasonCode` values. The internal `NupmCompatibility` enum is retained for classifier implementation, but the canonical report is built from the classifier result plus parsed metadata and observed filesystem facts.

| Internal compatibility | Outcome | Reason code(s) | Recommended action |
|------------------------|---------|----------------|------------------|
| `ImportableModule` | `importable_now` | none | `import` |
| `DeferredScript` | `manual_migration_required` | `script_package` or `auxiliary_scripts` | `manual_migration` |
| `UnsupportedDependencies` | `manual_migration_required` | `declared_dependencies` | `manual_migration` |
| `UnsupportedCustomBuild` | `manual_migration_required` | `custom_build_nu` | `manual_migration` |
| `UnknownType` | `unsupported` | `unknown_package_type` | `repair_source` |
| `InvalidMetadata` | `unsupported` | `missing_required_keys`, `unsupported_metadata_shape`, `unsupported_nuon_construct`, or `metadata_limit_exceeded` | `repair_source` |
| `UnsafeFilesystemLayout` | `unsupported` | `unsafe_filesystem_layout`, `missing_module_directory`, or `missing_module_entry` | `repair_source` |

Installed-only directories (no `nupm.nuon`) are reported separately with outcome `inspect_only` and reason `metadata_unavailable`.

### Status report buckets (Phase 6.1)

Separate counts — do not label installed-only as rejected:

```text
Migration outcomes:
  importable_now
  inspect_only
  manual_migration_required
  unsupported
Installed-only module directories (metadata unavailable; not import-eligible)
Script entries
Unsafe/unreadable entries
Numan nupm imports (lockfile origin nupm_import)
Source drift (imports): count where live nupm source differs from provenance (Phase 6.3)
Name overlap warnings (optional): nupm source declared name matches installed module under different scoped id
```

### Drift categories (Phase 6.3)

`numan nupm diff owner/name` compares lockfile + provenance against the live nupm source tree (read-only; exit 0 when drift is detected, exit 1 on compare errors):

| Status | Meaning |
|--------|---------|
| `Unchanged` | Metadata and source payload hashes match provenance |
| `SourceMissing` | Recorded `nupm_source_path` absent |
| `MetadataChanged` | `nupm.nuon` bytes hash differs |
| `PayloadChanged` | `<name>/` module tree manifest hash differs |
| `UnsafeSourceTreeChange` | Live tree no longer import-eligible or fails safety checks |
| `CannotCompare` | Not a nupm import or missing provenance |

### Manifest import (Phase 6.3)

```bash
numan nupm import --manifest PATH [--nupm-home PATH] [--yes]
```

TOML schema (paths relative to validated `NUPM_HOME`):

```toml
[[imports]]
source = "relative/to/nupm/home"
as = "owner/name"
```

Batch import is **all-or-nothing**: pre-flight classifies all entries; on any failure after staging begins, staged dirs are removed and the lockfile is unchanged.

### Import provenance (Phase 6.5)

`state/nupm-imports.json` is now version `2`. Each `NupmImportRecord` records:

- `original_nupm_name`: declared `name` from `nupm.nuon`.
- `original_nupm_version`: declared `version` from `nupm.nuon`.
- `selection_reason`: typed enum (e.g., `module_entry`) explaining why this source shape was selected.
- `transformation_performed`: typed enum (e.g., `copied_module_tree`) describing the actual transformation applied.

Version 1 records are loaded and upgraded with `original_nupm_name` / `original_nupm_version` set to `"unknown"` and the default selection/transformation values. Provenance is never written for packages that are not `importable_now`.

### JSON output schema (Phase 6.5)

`numan nupm status --json` and `numan nupm inspect --json` emit deterministic, sorted JSON with two version fields:

- `schema_version`: version of the JSON envelope shape (currently `1`).
- `compat_schema_version`: version of the nupm parser/classifier rules (currently `1`).

Example `status` JSON shape:

```json
{
  "schema_version": 1,
  "compat_schema_version": 1,
  "command": "status",
  "nupm_home": "/path/to/nupm",
  "home_not_configured": false,
  "modules_dir_present": true,
  "scripts_dir_present": true,
  "counts": {
    "importable_now": 1,
    "inspect_only": 0,
    "manual_migration_required": 2,
    "unsupported": 1,
    "installed_only": 0,
    "script_entries": 0,
    "unsafe_entries": 0
  },
  "source_roots": [
    {
      "name": "minimal-module",
      "source_path": "/path/to/package",
      "version": "0.1.0",
      "package_type": "module",
      "outcome": "importable_now",
      "reason_codes": ["none"],
      "recommended_action": "import",
      "detected_features": {
        "has_scripts": false,
        "has_dependencies": false,
        "has_build_script": false,
        "is_overlay": false
      }
    }
  ],
  "installed_only": [],
  "numan_nupm_imports": 0,
  "source_drift_count": 0,
  "name_overlap_count": 0
}
```

`outcome`, `reason_codes`, and `recommended_action` are stable snake_case identifiers. Consumers must tolerate unknown reason codes for forward compatibility.

### Schema evolution policy

- New reason codes can be added without bumping `schema_version` or `compat_schema_version`; consumers must tolerate unknown values.
- New fields in the JSON envelope require a `schema_version` bump.
- New classifier rules (e.g., new outcome mapping) require a `compat_schema_version` bump.
- `state/nupm-imports.json` version bumps only when persisted record structure changes; old versions are upgraded on load.

### Phase 6.1 non-goals

Phase 6.1 does **not**: write under `NUPM_HOME`; create lifecycle journals; acquire mutation lock; copy payloads; modify lockfile; run `nu` or `build.nu`; read/modify Nu config; activate packages.

---

## Package type classification (nupm behavior)

| `type` | nupm install behavior | Numan Phase 6 |
|--------|----------------------|---------------|
| `module` | Copy `<pkg>/<name>/` → `NUPM_HOME/modules/<name>/`; optional `scripts` → `scripts/` | **Importable** only if narrow profile passes (see below) |
| `script` | Install `<name>.nu` + optional `scripts` list → `scripts/` | **Rejected** (`DeferredScript`) |
| `custom` | Run `nu build.nu <path/to/nupm.nuon>` from temp dir | **Rejected** (`UnsupportedCustomBuild`); never execute `build.nu` |
| other | Error from nupm | **Rejected** (`UnknownType`) |

### `build.nu` detection

- **custom** type: nupm requires `build.nu` in package root; executes it via `^$nu.current-exe`.
- **module** / **script** type: nupm ignores `build.nu` even if present.
- **Numan rule:** reject import if `build.nu` exists **regardless of type** (stricter than nupm; avoids ambiguous trees).

---

## Supported format profile (Numan import)

A package is **import-eligible** when **all** conditions hold:

```text
metadata file:     nupm.nuon (parseable within NUON subset)
declared type:     module
required keys:     name, version, type present
module directory:  <pkg_root>/<name>/ exists
module entry:      <pkg_root>/<name>/mod.nu exists
dependencies:      no deps / dependencies / requires field in metadata
scripts field:     absent (auxiliary script install is out of scope)
build.nu:          absent at package root
filesystem:        regular files and directories only; no symlink escape
payload:           all module files under package root (no external imports in metadata)
identity:          user supplies --as owner/name (never derived from nupm name)
```

### Supported optional metadata fields (ignored for import logic)

`description`, `license` — parsed and displayed in `inspect`; not required for import eligibility.

`name`, `version`, and `type` remain **required** (nupm `open-package-file`). `version` is validated and shown in `inspect` but does not drive import selection.

---

## Rejected format profile

| Condition | Compatibility class | Outcome | Reason code(s) |
|-----------|---------------------|---------|----------------|
| Unparseable NUON / closure / `$var` | `InvalidMetadata` | `unsupported` | `unsupported_nuon_construct` |
| Missing `name`, `version`, or `type` | `InvalidMetadata` | `unsupported` | `missing_required_keys` |
| `type: script` | `DeferredScript` | `manual_migration_required` | `script_package` |
| `type: custom` | `UnsupportedCustomBuild` | `manual_migration_required` | `custom_build_nu` |
| Unknown `type` | `UnknownType` | `unsupported` | `unknown_package_type` |
| `deps`, `dependencies`, or `requires` field | `UnsupportedDependencies` | `manual_migration_required` | `declared_dependencies` |
| `scripts: [...]` on module | `DeferredScript` | `manual_migration_required` | `auxiliary_scripts` |
| `build.nu` present | `UnsupportedCustomBuild` | `manual_migration_required` | `custom_build_nu` |
| Missing `<name>/` directory | `UnsafeFilesystemLayout` | `unsupported` | `missing_module_directory` |
| Missing `mod.nu` | `UnsafeFilesystemLayout` | `unsupported` | `missing_module_entry` |
| Symlink / reparse-point escape | `UnsafeFilesystemLayout` | `unsupported` | `unsafe_filesystem_layout` |
| Path outside package root | `UnsafeFilesystemLayout` | `unsupported` | `unsafe_filesystem_layout` |

Rejected packages remain **read-only visible** in `status` / `inspect`; Numan never mutates nupm trees. Import is only permitted for `importable_now` outcomes.

---

## Fixture package inventory

Corpus root: `tests/fixtures/nupm/`.

### Supported (`supported/`)

| Fixture | Layout | Expected Phase 6.1 class |
|---------|--------|------------------------|
| `minimal-module/` | Standard module; bare identifiers; no optional fields | `ImportableModule` |
| `module-with-metadata/` | Adds `description`, `license` | `ImportableModule` |

### Rejected (`rejected/`)

| Fixture | Based on | Expected class | Outcome | Reason code |
|---------|----------|----------------|---------|-------------|
| `script-type/` | nupm `spam_script` | `DeferredScript` | `manual_migration_required` | `script_package` |
| `custom-with-build/` | nupm `spam_custom` | `UnsupportedCustomBuild` | `manual_migration_required` | `custom_build_nu` |
| `custom-without-build/` | custom type, no build.nu | `UnsupportedCustomBuild` | `manual_migration_required` | `custom_build_nu` |
| `missing-module-dir/` | module without `<name>/` dir | `UnsafeFilesystemLayout` | `unsupported` | `missing_module_directory` |
| `missing-mod-nu/` | module dir without mod.nu | `UnsafeFilesystemLayout` | `unsupported` | `missing_module_entry` |
| `module-with-scripts/` | nupm `spam_module` | `DeferredScript` | `manual_migration_required` | `auxiliary_scripts` |
| `unknown-type/` | fictional `overlay` type | `UnknownType` | `unsupported` | `unknown_package_type` |
| `malformed-closure/` | closure in metadata | `InvalidMetadata` | `unsupported` | `unsupported_nuon_construct` |
| `external-deps/` | `deps` record | `UnsupportedDependencies` | `manual_migration_required` | `declared_dependencies` |
| `missing-required-keys/` | no `name` key | `InvalidMetadata` | `unsupported` | `missing_required_keys` |

### Layout sample (`nupm-home-layout/`)

Simulates **post-install** `$NUPM_HOME` layout (matches nupm `install-path` for modules: only the inner module tree is copied; `nupm.nuon` stays at the source package root):

```text
nupm-home-layout/
  modules/minimal-module/
    mod.nu                    ← installed module payload only (no nupm.nuon)
  scripts/example-script.nu   ← sample script install; not imported
```

Phase 6.1 discovery under `--nupm-home` must treat `modules/<name>/` as an installed module candidate even when `nupm.nuon` is absent. Such trees are **not import-eligible** until paired with a source package root that contains metadata (use `supported/*` fixtures for import tests).

Use as `--nupm-home` target in Phase 6.1 status/discovery integration tests.

---

## NUPM_HOME discovery (Numan commands)

Numan **must not** guess nupm's default config path.

### When `--nupm-home` / `NUPM_HOME` is required

Resolution order applies to commands that scan an nupm installation tree:

```text
numan nupm status
numan nupm inspect --all
numan nupm inspect <PACKAGE-PATH> [--exit-on-ineligible]
numan nupm import --manifest PATH   (manifest paths relative to nupm home)
```

Order:

1. `--nupm-home PATH`
2. `NUPM_HOME` environment variable
3. Error with guidance (no silent fallback)

### When an explicit source path is enough

These forms take a **package source root or path inside it** and do **not** require `--nupm-home` or `NUPM_HOME`:

```text
numan nupm inspect <PACKAGE-PATH>
numan nupm import <PACKAGE-PATH> --as OWNER/NAME
```

Discovery walks parents from `<PACKAGE-PATH>` to locate `nupm.nuon` (same as nupm `find-root`). Use `tests/fixtures/nupm/supported/*` for import-eligible source trees.

---

## Platform path behavior

| Topic | Windows | Linux | macOS |
|-------|---------|-------|-------|
| Path separators | `\` in UI; normalize for comparisons | `/` | `/` |
| `NUPM_HOME` | User-set; often under `%LOCALAPPDATA%` or `%APPDATA%` | XDG or `~/.config/nupm` | Same as Linux |
| Symlinks | Reject reparse-point / junction escapes on copy | Reject symlink escape | Reject symlink escape |
| Unicode paths | Must round-trip in inspect output | Same | Same |
| Case sensitivity | Case-insensitive FS common; compare paths canonically | Case-sensitive | Case-sensitive default |

Phase 6.0 fixtures use ASCII paths; Phase 6.4 adds Unicode / space acceptance tests (T24) and real-Nu `#[ignore]` acceptance tests (`tests/nupm_real_nu_test.rs`).

### Inspect exit codes (Phase 6.4)

By default, `inspect` exits 0 even when packages are ineligible (informational output). Pass `--exit-on-ineligible` to exit 1 when any candidate is not `ImportableModule`.

### Metadata parser fuzz (Phase 6.4)

`parse_metadata` is exercised against 10k+ arbitrary byte sequences (`t05_arbitrary_bytes_no_panic`) and must never panic; successful parses must satisfy `validate_invariants()`.

---

## Known unsupported nupm features (Phase 6 scope)

The following nupm capabilities are **explicitly out of scope** for Phase 6. They must be classified and rejected safely, never partially emulated:

| Feature | nupm support | Numan Phase 6 |
|---------|--------------|---------------|
| Script packages | `type: script` | Inspect only; no import |
| Custom / `build.nu` installs | Executes Nu build hook | Detect; never run |
| Plugin packages (`nu_plugin_*`) | Registry lists git plugins | No import (plugin path is registry-only in Numan) |
| Registry fetch / git clone | `fetch-package`, `download-pkg` | No network reads of nupm registries |
| Overlays | Design doc | Not scanned |
| Module `scripts:` auxiliary install | Copies to `scripts/` | Reject when field present |
| Dependency resolution | Not in metadata today | Reject if `deps` appears |
| Bidirectional sync | nupm owns NUPM_HOME | Read-only toward nupm; one-way import into Numan |
| `nupm publish` / registry writes | nupm command | Not invoked |
| Activations / packages.nuon config | nupm user config | Not read or written |

---

## Test matrix (Phase 6.1+)

Phase 6.0 defines expectations; tests land in Phase 6.1–6.4.

| ID | Area | Input | Expected |
|----|------|-------|----------|
| T01 | Parser | `supported/minimal-module/nupm.nuon` | Parse OK; name/type/version extracted |
| T02 | Parser | `supported/module-with-metadata/nupm.nuon` | Optional fields preserved |
| T03 | Parser | `rejected/malformed-closure/nupm.nuon` | Err InvalidMetadata |
| T04 | Parser | `rejected/missing-required-keys/nupm.nuon` | Err InvalidMetadata |
| T05 | Parser | bounded property corpus | No panic; bounded runtime/allocation; Ok satisfies invariants; known-invalid mutations Err |
| T06 | Classify | `supported/minimal-module/` | ImportableModule |
| T07 | Classify | `rejected/script-type/` | DeferredScript |
| T08 | Classify | `rejected/custom-with-build/` | UnsupportedCustomBuild |
| T09 | Classify | `rejected/module-with-scripts/` | DeferredScript |
| T10 | Classify | `rejected/external-deps/` | UnsupportedDependencies |
| T11 | Classify | `rejected/missing-mod-nu/` | UnsafeFilesystemLayout |
| T12 | Classify | `rejected/unknown-type/` | UnknownType |
| T13 | Discovery | `nupm-home-layout/` + `--nupm-home` | Detects installed `modules/minimal-module/` without `nupm.nuon`; not import-eligible |
| T14 | Discovery | `inspect --all` without home | Actionable error; `status` without home exits 0 with guidance |
| T15 | Safety | inspect/status on fixtures | Fixture manifest unchanged (SHA-256, not mtime) |
| T16 | Import | supported module (Phase 6.2) | Payload under `$NUMAN_ROOT` only |
| T17 | Import | any rejected fixture (Phase 6.2) | Error; nupm bytes unchanged |
| T18 | Drift | `numan nupm diff` after source edit (Phase 6.3) | Reports `PayloadChanged`; status drift count increments |
| T19 | Import | stale nupm import journal (Phase 6.2) | Retry blocked until `numan gc` |
| T20 | Drift | source edit without re-import (Phase 6.2) | Installed revision unchanged |
| T21 | Re-import | modify source + `--yes` (Phase 6.3) | New `revision_id`; old payload gc-eligible |
| T22 | Manifest | `--manifest` batch (Phase 6.3) | All entries committed atomically |
| T23 | Activation | import + `numan activate` (Phase 6.3) | Managed autoload + lockfile `module_activation` |
| T24 | Platform | Unicode path (Phase 6.4) | Inspect + import |
| T25 | Platform | symlink in module tree (Phase 6.4, Unix) | Import rejected; lockfile unchanged |
| T26 | Inspect | `--exit-on-ineligible` on rejected fixture | Exit 1 |
| T27 | Parser | 10k arbitrary byte fuzz (Phase 6.4) | No panic |
| T28 | Real-Nu | imported module autoload (Phase 6.4, `#[ignore]`) | `nu -n` passes on generated autoload |
| T29 | Assessment | `custom-without-build/` | `manual_migration_required` / `custom_build_nu` |
| T30 | Assessment | `missing-module-dir/` | `unsupported` / `missing_module_directory` |
| T31 | Assessment | `missing-required-keys/` | `unsupported` / `missing_required_keys` |
| T32 | Assessment | `unsupported-nuon-construct/` | `unsupported` / `unsupported_nuon_construct` |
| T33 | JSON | `status --json` and `inspect --json` | `schema_version` and `compat_schema_version` present; deterministic output |
| T34 | Import | `rejected/script-type/` with `--yes` | Refused; no lockfile/provenance/payload changes |
| T35 | Provenance | `supported/minimal-module/` import | `original_nupm_name`, `original_nupm_version`, `selection_reason`, `transformation_performed` recorded |

---

## Audit conclusions

1. **Narrow support is justified.** At the pinned commit, nupm itself only ships five `nupm.nuon` examples; the official test corpus covers three types. Numan Phase 6 targets **module-only**, **no build.nu**, **no scripts field**, **no deps**.
2. **Parser grammar is bounded.** Real files use records, bare words, strings, and string lists without exotic NUON literals.
3. **Layout rule is strict and simple:** `<root>/nupm.nuon` + `<root>/<name>/mod.nu`.
4. **Numan is stricter than nupm** on `build.nu` presence and `scripts` metadata to avoid partial script installs.
5. **Fixture corpus is ready** for Phase 6.1 metadata parser and classifier unit tests.

---

## Changelog

| Date | Change |
|------|--------|
| 2026-06-28 | Initial Phase 6.0 audit; pin `421eee1c`; fixture corpus |
| 2026-06-29 | compat-schema-v1: field-specific grammar, BehaviorFlags, classifier pipeline, status buckets, Phase 6.1 non-goals |
| 2026-06-28 | Phase 6.3: drift engine, `numan nupm diff`, manifest import, re-import polish, activation tests |
| 2026-06-28 | Phase 6.4: `--exit-on-ineligible`, parser fuzz, Unicode/symlink tests, real-Nu acceptance |
| 2026-07-01 | Phase 6.5: migration planner outcomes (`importable_now`, `inspect_only`, `manual_migration_required`, `unsupported`), reason codes, `--json` reporting, provenance v2 |
