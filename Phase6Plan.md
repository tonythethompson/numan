# Phase 6 Proposal: Safe nupm Coexistence and Explicit Migration

## Objective

Phase 6 adds safe interoperability with nupm without making Numan dependent on nupm's mutable layouts, configuration conventions, or evolving metadata format.

The phase provides:

* read-only discovery of a user-designated nupm installation;
* inspection and classification of nupm packages;
* explicit one-way import of supported local nupm modules into Numan's immutable revision store;
* provenance and drift tracking for imported packages;
* safe coexistence between Numan's managed vendor-autoload file and nupm's module and script directories.

Phase 6 does **not** synchronize state bidirectionally, modify nupm-managed files, execute `build.nu`, or pretend that nupm's current format is a stable package-manager API.

> **Prerequisite note**: Phase 5 update/remove/gc (PR #6) must be merged before Phase 6 begins. Phase 5 source builds (5.2) and plugin gate (5.5) are **not** required.

---

# 1. Scope

## In scope

* Read-only nupm home discovery.
* Read-only package inspection and compatibility reporting.
* Parsing supported `nupm.nuon` metadata without executing Nushell package code.
* Explicit import of compatible module packages into Numan.
* Imported-package provenance, content identity, and source-tree drift checks.
* Activation of imported modules through Phase 4's existing managed vendor-autoload path.
* Explicit re-import as a new immutable revision.
* Coexistence diagnostics for Numan and nupm installations.

## Explicitly out of scope

* Writing to `NUPM_HOME`.
* Editing `env.nu`, `config.nu`, `$env.NU_LIB_DIRS`, `NU_LIB_DIRS`, or `$env.PATH`.
* Running `nupm install`, `nupm update`, `nupm uninstall`, or other nupm mutations.
* Running arbitrary `build.nu`.
* Importing custom nupm package types.
* Importing or activating nupm scripts.
* Importing plugins built or installed through nupm.
* Bidirectional sync.
* Generating nupm overlays.
* Generating or editing `nupm.nuon`.
* Treating imported packages as registry-verified.
* Automatic import at `numan init`.
* Automatic re-import when the source tree changes.

---

# 2. Core Principles

1. **Numan and nupm retain separate ownership domains.**

   ```text
   Numan owns: $NUMAN_ROOT and its one managed Nu vendor-autoload file.
   nupm owns: NUPM_HOME, its metadata, overlays, modules, scripts, and configuration model.
   ```

2. **nupm interoperability is read-only until explicit import.**

3. **Import is a snapshot, not a live link.**

   Once imported, Numan uses its immutable payload copy. Later edits or updates under the nupm source path do not affect Numan automatically.

4. **No implicit trust upgrade.**

   Imported nupm content is local user-supplied content. It is never labeled registry-signed, registry-verified, or reproducible from a Numan registry.

5. **No unsupported package execution.**

   `build.nu`, custom project types, scripts, and unknown metadata are classified but never run.

6. **Canonical Numan IDs are required.**

   nupm metadata may not provide a Numan-compatible scoped ID. Numan never invents one.

7. **Imported modules activate namespaced by default.**

   Imported modules use Phase 4's `module` import mode only. `all` import mode is not available for nupm imports in Phase 6.

8. **No automatic dependency translation.**

   External dependencies declared or inferred from nupm packages remain unsupported until Numan has an explicit runtime module-linking contract.

---

# 3. Preconditions

Phase 6 requires Phase 4 (complete) and Phase 5 update/remove/gc (PR #6). It does **not** require Phase 5 source builds (5.2), lockfile snapshot UI (5.3), or plugin gate (5.5).

Specific dependencies:

```text
Phase 4 (complete):
  managed vendor-autoload activation — nu/autoload.rs
  module activation state and recovery — state/autoload_journal.rs
  external-file drift protection — util/fs_safety.rs
  root mutation lock — acquire_mutation_lock()
  FakeCandidateRunner — nu/autoload.rs (test seam for module validation)

Phase 5 update/remove/gc (PR #6 — must be merged first):
  LockfileEntry v2 fields: revision_id, payload_sha256,
    executable_sha256, selection_reason, origin
  compute_revision_id(payload_dir) — state/lockfile.rs
  PendingLifecycle journal — state/lifecycle_journal.rs
  LifecycleOp and LifecycleStage enums (will be extended in Phase 6)
  write_json_atomic — util/atomic.rs
  Lockfile::snapshot() before mutation
```

---

# 4. Compatibility Contract

Phase 6 supports only this initial nupm package profile:

```text
package type: module
metadata file: nupm.nuon
module entry: package directory containing mod.nu
dependencies: none outside the imported payload
custom build: absent
source tree: regular files and safe directories only
```

A package is rejected for import when any of the following are true:

```text
unknown or malformed metadata
custom type
build.nu present
script type
plugin type
external dependency metadata
symlink or reparse-point escape
missing mod.nu
entry path outside package root
unscoped or conflicting requested Numan identity
```

Rejected packages are shown in inspection output with an explicit reason.

---

# 5. Phase 6.0: nupm Compatibility Audit

Before implementation, add a compatibility audit against a pinned nupm commit and fixture corpus.

The audit must establish:

```text
supported nupm.nuon field shapes
actual package directory layouts
module entry conventions
NUPM_HOME discovery expectations
module versus script versus custom classification
build.nu detection behavior
dependency metadata variants
symlink handling
Windows, Linux, and macOS path behavior
```

The audit produces a checked-in compatibility document:

```text
docs/nupm-compatibility.md
```

It must include:

```text
supported format profile
rejected format profile
fixture package inventory
pinned nupm commit
known unsupported features
test matrix
```

No Phase 6 import implementation begins until this audit defines a narrow supported profile.

---

# 6. Command Surface

```text
numan nupm status [--nupm-home PATH]
numan nupm inspect <PACKAGE-PATH> [--nupm-home PATH]
numan nupm inspect --all [--nupm-home PATH]
numan nupm import <PACKAGE-PATH> --as OWNER/NAME [--yes]
numan nupm import --manifest PATH [--nupm-home PATH] [--yes]
numan nupm diff <OWNER/NAME>
```

## 6.0 CLI architecture

`numan nupm` is a **subcommand group** — a `Commands::Nupm(NupmArgs)` variant in `main.rs` where `NupmArgs` contains its own nested `NupmCommands` enum:

```rust
// src/cmd/nupm.rs
#[derive(Parser)]
pub struct NupmArgs {
    #[command(subcommand)]
    pub command: NupmCommands,
}

#[derive(Subcommand)]
pub enum NupmCommands {
    Status(StatusArgs),
    Inspect(InspectArgs),
    Import(ImportArgs),
    Diff(DiffArgs),
}
```

`main.rs` adds one arm: `Commands::Nupm(args) => cmd::nupm::execute(&args, &root)`.

`cmd/nupm.rs` dispatches to `nupm_compat::*` functions — it does not contain domain logic itself.

## 6.1 `numan nupm status`

Read-only.

Resolution order for nupm home:

```text
1. --nupm-home PATH (explicit flag)
2. NUPM_HOME environment variable
3. error — no automatic guess
```

For reference, nupm's own default locations (Numan must NOT auto-use these):

```text
Linux/macOS: $XDG_DATA_HOME/nupm  or  ~/.local/share/nupm
Windows:     %APPDATA%\nupm
```

Documenting the defaults here helps implementers write accurate error messages that guide users. Numan must not read these paths without explicit user opt-in.

If no location is available:

```text
No nupm home was supplied.

Pass --nupm-home <path> or set NUPM_HOME for this command.
Numan will not guess or modify nupm's installation location.
```

Output includes:

```text
nupm home path
existence and safety status
detected modules directory
detected scripts directory
supported package count
unsupported package count
existing Numan import records
source drift count
```

## 6.2 `numan nupm inspect`

Read-only.

Inspects a specified package path or all discoverable package candidates under the designated nupm home.

For each candidate, show:

```text
source path
metadata status
declared name
declared type
entry-point status
dependency status
build.nu status
import eligibility
suggested import command
```

Example:

```text
foo
  Source:       C:\Users\me\AppData\Local\nupm\modules\foo
  Type:         module
  Metadata:     supported
  Entry:        foo/mod.nu
  Dependencies: none
  Build hook:   absent
  Eligible:     yes
  Import:       numan nupm import <path> --as owner/foo
```

## 6.3 `numan nupm import`

Imports one compatible nupm module as an immutable Numan revision.

Required identity:

```text
--as owner/name
```

Numan must not derive a synthetic owner such as `nupm/foo`.

Import flow:

1. Acquire root mutation lock.

2. Recover any pending lifecycle journal.

3. Parse and classify source metadata.

4. Validate eligibility.

4a. Check for existing lockfile entry under `owner/name`. If one exists:
    - If `origin != "nupm_import"`: bail — would overwrite a registry-installed package. User must remove it first.
    - If `origin == "nupm_import"`: treat as re-import (see §10). Require `--yes` even with matching identity.

5. Show consent:

   ```text
   Local nupm module import

     Source path:       ...
     Target Numan ID:   owner/name
     Package type:      module
     Entry point:       mod.nu
     Trust level:       local foreign import
     Build scripts:     not executed
     Activation:        not performed
   ```

6. Create operation-scoped staging under `$NUMAN_ROOT`.

7. Copy approved payload files into staging.

8. Reject unsafe files, path traversal, symlink escapes, and reparse points.

9. Calculate deterministic payload manifest hash.

10. Validate the imported module entry through Phase 4's candidate-validation path.

11. Promote to immutable Numan revision storage.

12. Write lifecycle journal progress.

13. Add the imported package as a direct root with explicit import intent.

14. Persist provenance.

15. Do not activate automatically.

16. Clear journal.

`--yes` flag: skips the consent prompt. Without `--yes`, abort after displaying the consent block unless the user confirms. For scripting/CI, `--yes` is required.

Imported revision `LockfileEntry` fields set at import time:

```text
origin:           "nupm_import"
selection_reason: "explicit_nupm_import"
payload_sha256:   computed from copied payload
revision_id:      compute_revision_id(staged_payload_dir)
git_url:          observed remote URL (descriptive only, may be null)
git_rev:          observed HEAD commit (descriptive only, may be null)
installed_at:     import timestamp
```

The remaining nupm-specific provenance (source path, metadata hash, trust level) is stored in `state/nupm-imports.json` via `state/nupm_import.rs` — not in the lockfile. Lockfile stays schema-stable; provenance file is nupm-specific and versioned separately.

## 6.4 Bulk import manifest

Bulk import requires an explicit mapping file.

Example:

```toml
[[imports]]
source = "modules/foo"
as = "owner/foo"

[[imports]]
source = "modules/bar"
as = "owner/bar"
```

Each mapping is independently validated, but resolution, staging, promotion, and lockfile selection occur as one lifecycle transaction.

If any mapped package is unsupported or invalid, do not promote any payload.

## 6.5 `numan nupm diff`

Read-only.

Compares an imported Numan revision against its recorded nupm source snapshot.

Output categories:

```text
unchanged
source missing
metadata changed
payload changed
unsafe source-tree change
cannot compare
```

`diff` never changes Numan's selected revision or nupm files.

A changed source tree requires explicit re-import.

---

# 7. Metadata Parsing

## 7.1 Parsing rules

`nupm.nuon` is metadata, not executable package code.

Phase 6 must parse supported metadata through a **pure-Rust constrained NUON parser** implemented in `nupm_compat/metadata.rs`. Do not shell out to the Nu binary — it introduces a trust boundary issue and a hard runtime dependency.

It must never:

```text
source nupm.nuon
use nupm.nuon
overlay use nupm.nuon
execute package commands
execute build.nu
invoke nu binary to parse metadata
```

The parser receives only the metadata bytes and returns a normalized internal descriptor.

## 7.1a NUON subset to support

NUON is a superset of JSON with additional Nushell-native literals (dates, binary data, ranges, closures, etc.). Phase 6 supports only the narrow subset that actually appears in nupm.nuon files:

```text
Supported:
  record literal       { key: value, ... }
  double-quoted string "value"
  bare-word type tags  module | script | custom
  boolean              true | false
  integer              42
  null                 null
  list of strings      ["a", "b"]
  nested record        { name: "foo", deps: { bar: "1.0" } }

Rejected at parse time (treat as InvalidMetadata):
  closures             { || ... }
  dates                2024-01-01
  binary               0x[DEAD]
  ranges               0..10
  any value beginning with $ or ^
```

The parser is hand-written recursive descent, approximately 200-300 lines. No external NUON crate is required. The audit (§5) must confirm the actual field shapes observed in real nupm.nuon files before the parser grammar is finalized.

Fixture-driven approach: write the parser to pass the fixture corpus, not to be a general NUON parser. Any fixture file that requires features outside the supported subset documents a rejected case.

The parser must:
- be fuzz-tested with arbitrary byte sequences (no panic on malformed input)
- return `Err` with a specific reason for every rejection
- never allocate unbounded structures (cap list length at 1000, string length at 64 KB)

## 7.2 Normalized descriptor

```rust
pub struct NupmPackageDescriptor {
    pub source_path: PathBuf,
    pub metadata_path: PathBuf,
    pub metadata_sha256: String,

    pub declared_name: String,
    pub declared_type: NupmPackageType,

    pub module_entry: Option<PathBuf>,
    pub has_build_script: bool,

    pub declared_dependencies: BTreeMap<String, String>,
    pub compatibility: NupmCompatibility,
}
```

```rust
pub enum NupmCompatibility {
    ImportableModule,
    DeferredScript,
    UnsupportedCustomBuild,
    UnsupportedDependencies,
    InvalidMetadata,
    UnsafeFilesystemLayout,
    UnknownType,
}
```

---

# 8. Imported Revision Provenance

Provenance is split across two storage locations:

**In `LockfileEntry`** (already-existing fields, set at import time):

```text
origin:           "nupm_import"
selection_reason: "explicit_nupm_import"
payload_sha256:   SHA256 of the imported payload archive (staged copy)
revision_id:      compute_revision_id(promoted payload dir)
git_url:          observed git remote URL (or null)
git_rev:          observed HEAD commit hash (or null)
installed_at:     import timestamp
```

**In `state/nupm-imports.json`** (managed by `state/nupm_import.rs`):

This file holds nupm-specific fields that have no place in the general `LockfileEntry` schema. Keyed by `owner/name`.

```json
{
  "version": 1,
  "imports": {
    "owner/name": {
      "trust_level": "local_foreign_import",
      "nupm_source_path": "C:/...",
      "nupm_metadata_path": "C:/.../nupm.nuon",
      "nupm_metadata_sha256": "...",
      "source_payload_sha256": "...",
      "imported_payload_sha256": "...",
      "observed_git_remote": null,
      "observed_git_commit": null,
      "imported_at": "..."
    }
  }
}
```

`nupm-imports.json` uses `write_json_atomic`. Entries are removed when `numan remove owner/name` removes the lockfile entry.

Optional Git information is descriptive only. It does not convert a local import into registry provenance or source-build provenance.

---

# 9. Activation and Coexistence

Imported modules activate through the normal Phase 4 vendor-autoload mechanism.

Rules:

```text
default import mode: module
all-members import mode: unavailable
external package dependencies: unsupported
internal relative imports within imported payload: supported if candidate validation succeeds
```

Numan does not modify `NU_LIB_DIRS`, `PATH`, or nupm overlays.

It generates `use` statements with validated absolute paths into Numan's immutable payload store. That means activation does not depend on the continuing existence of `NUPM_HOME`.

If a nupm package remains active through the user's own environment configuration, Numan reports potential command-name overlap but does not attempt to disable or rewrite nupm state.

`numan list` must show imported packages with their origin. Suggested display:

```text
owner/name  1.0.0  module  (nupm import)
```

The `(nupm import)` tag comes from `origin == "nupm_import"` in the lockfile entry. The list command is modified to check `origin` and annotate accordingly.

---

# 10. Re-import Policy

There is no live synchronization.

To adopt local nupm changes:

```text
numan nupm import <path> --as owner/name --yes
```

When the source payload hash differs:

* create a new retained immutable Numan revision;
* show old and new `revision_id` values and source payload hashes;
* preserve the prior selected revision until the lifecycle transaction commits;
* require the `--yes` flag (same as initial import) before promoting the new revision;
* retain the old revision for garbage-collection rules.

Numan never writes an updated revision back into nupm.

---

# 11. Journaling and Recovery

## 11.1 Journal changes required

Phase 6 extends `state/lifecycle_journal.rs` from Phase 5:

```rust
// Add to LifecycleOp enum:
NupmImport,

// Add to LifecycleStage enum:
PayloadsStaged,      // staging dirs created and filled
PayloadsPromoted,    // staging dirs renamed to immutable paths
SelectionCommitted,  // lockfile and nupm-imports.json written
```

`PendingLifecycle` gains nupm-import-specific optional fields (all `#[serde(default)]`):

```rust
pub nupm_source_path: Option<String>,
pub nupm_metadata_sha256: Option<String>,
pub staging_dir: Option<String>,       // relative to root
pub promoted_payload_path: Option<String>, // relative to root
```

Existing `Update` and `Remove` operations do not use the new fields. `check_stale_journal()` already handles unknown ops gracefully via `LifecycleOp`'s `#[serde(rename_all = "snake_case")]` — adding a new variant is backward-compatible for reads of old journals.

## 11.2 Import journal flow

nupm import journals with:

```text
op:                   nupm_import
package_id:           owner/name
stage:                Prepared → PayloadsStaged → PayloadsPromoted → SelectionCommitted
nupm_source_path:     recorded for recovery diagnostics
nupm_metadata_sha256: recorded for reconciliation
staging_dir:          path of staging dir (relative to root)
promoted_payload_path: path after promotion (relative to root)
```

Recovery rules:

* `Prepared`: no payload written. Safe to clear journal; re-run import from scratch.
* `PayloadsStaged`: staging dir exists but not promoted. Clean staging dir; re-run.
* `PayloadsPromoted`: payload is in immutable storage but lockfile not updated. The orphaned payload will be collected by `numan gc`. Clear journal; re-run import (will detect version-match and reuse or create new revision).
* `SelectionCommitted`: operation is complete. Clear journal.
* Recovery never re-reads mutable nupm source files to reconstruct an interrupted transaction.
* If source provenance cannot be reconciled, block mutation and preserve journal evidence.

---

# 12. Files

## New files

```text
src/nupm_compat/mod.rs
src/nupm_compat/discovery.rs
src/nupm_compat/metadata.rs
src/nupm_compat/classify.rs
src/nupm_compat/import.rs
src/nupm_compat/drift.rs
src/cmd/nupm.rs
src/state/nupm_import.rs
tests/nupm_compat_test.rs
tests/fixtures/nupm/
docs/nupm-compatibility.md
```

## Modified files

```text
src/state/lifecycle_journal.rs  — add NupmImport op + PayloadsStaged/Promoted/SelectionCommitted stages
src/cmd/list.rs                 — annotate origin == "nupm_import" entries
src/cmd/mod.rs                  — pub mod nupm
src/main.rs                     — Commands::Nupm variant + dispatch arm
AGENTS.md
CLAUDE.md
```

Note: `src/core/package.rs`, `src/state/lockfile.rs`, and `src/install/transaction.rs` are **not** modified — all new fields are handled by Phase 5's existing v2 schema. nupm-import provenance lives in `nupm_import.rs`, not the lockfile.

---

# 13. Delivery Sequence

## Phase 6.0: Compatibility Audit

* Pin nupm source revision.
* Build fixture corpus.
* Define supported metadata and package profile.
* Document unsupported behavior.

## Phase 6.1: Read-only Discovery

* Implement `status`.
* Implement `inspect`.
* Implement safe filesystem scanning.
* Implement metadata parsing and classification.

No import or activation changes.

## Phase 6.2: One-Way Module Import

* Implement explicit `--as owner/name`.
* Implement staging, payload hashing, provenance, and lifecycle journal integration.
* Import supported modules only.
* Keep all imported modules inactive.

## Phase 6.3: Activation and Drift

* Allow imported modules to use standard Phase 4 activation.
* Implement `numan nupm diff`.
* Add explicit re-import behavior.
* Add coexistence diagnostics.

## Phase 6.4: Acceptance and Documentation

* Run fixture, unit, and integration suites.
* Test Windows, Linux, and macOS paths.
* Verify that no command modifies nupm state.
* Publish the compatibility matrix and known limitations.

---

# 14. Test Plan

## Unit tests

* `NUPM_HOME` override handling.
* Explicit `--nupm-home` precedence.
* No-home error behavior produces clear message with example flag usage.
* Metadata parser accepts all supported fixture files.
* Metadata parser rejects executable or malformed forms with specific reason.
* Metadata parser does not panic on arbitrary byte sequences (fuzz target).
* Module classification succeeds only with expected entry point.
* Scripts, custom packages, and build hooks are rejected.
* Scoped identity (`--as owner/name`) is required; bare name fails.
* Naming collision with existing registry package fails with actionable error.
* Payload manifest hash is deterministic across platforms.
* Symlink and reparse-point escape is rejected at file-copy time.
* Imported revision provenance round-trips through `nupm-imports.json`.
* `nupm-imports.json` entry removed when `numan remove owner/name` is called.
* Journal stage transitions are correct for nupm_import op.
* No nupm state mutation occurs during status, inspect, diff, or failed import.
* Exit code is 0 for success and non-zero for any error.

## Integration tests

* Import a supported local module.
* Verify imported payload is under `$NUMAN_ROOT`, not linked to nupm.
* Verify source-tree change does not alter imported revision.
* Verify explicit re-import creates a new retained revision.
* Verify a failed import preserves prior state.
* Verify interrupted import recovery.
* Verify nupm source removal after import does not break active imported module.
* Verify module activation through Numan's vendor-autoload file.
* Verify imported module defaults to namespaced activation.
* Verify nupm directories and metadata remain byte-for-byte unchanged.

## Real-Nu acceptance tests

* Imported module loads in a fresh Nu session.
* Paths with spaces, Unicode, and quotes work.
* Internal relative imports work when fully contained in the imported payload.
* Unsupported external imports fail candidate validation.
* nupm and Numan modules can coexist without Numan modifying Nu environment configuration.
* Windows, Linux, and macOS all pass.

---

# 15. Exit Codes

All `numan nupm` commands use the same exit code convention as the rest of Numan (anyhow propagates to `process::exit(1)` via main.rs):

```text
0  success
1  any error (parse failure, ineligible package, journal conflict, I/O error)
```

Specific `inspect` behavior: ineligible packages are shown in output but do not cause a non-zero exit unless `--exit-on-ineligible` is added in a future revision. The current default is informational output only.

`status` exits 0 even if no nupm home is configured — it prints the "no home" message to stdout and exits 0 (not an error, just a report). If `--nupm-home` is given but the path does not exist, exit 1.

---

# 16. Definition of Done

Phase 6 is complete when:

* Numan can inspect an explicitly supplied nupm installation without changing it.
* Numan imports only supported local nupm modules.
* Every import has explicit scoped identity and immutable payload provenance.
* Imported payloads are independent from later nupm changes.
* Imported modules show in `numan list` with `(nupm import)` origin tag.
* Imported modules can activate through existing Numan module activation.
* nupm scripts, custom builds, plugins, unknown formats, and external dependencies are rejected safely.
* No command writes to `NUPM_HOME`, nupm metadata, or Nu user configuration.
* Naming collision with registry package fails with actionable error.
* `nupm-imports.json` provenance is written atomically and cleaned up on remove.
* Drift is visible through `numan nupm diff`.
* Re-import is explicit, lifecycle-journaled, and requires `--yes`.
* NUON parser does not panic on arbitrary input (fuzz target passes).
* Fixture and real-Nu acceptance tests pass on Windows, Linux, and macOS.
* Bidirectional synchronization remains explicitly deferred.
