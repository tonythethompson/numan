# Phase 6 Proposal: Safe nupm Coexistence and Explicit Migration

## Objective

Phase 6 adds safe interoperability with nupm without making Numan dependent on nupm’s mutable layouts, configuration conventions, or evolving metadata format.

The phase provides:

* read-only discovery of a user-designated nupm installation;
* inspection and classification of nupm packages;
* explicit one-way import of supported local nupm modules into Numan’s immutable revision store;
* provenance and drift tracking for imported packages;
* safe coexistence between Numan’s managed vendor-autoload file and nupm’s module and script directories.

Phase 6 does **not** synchronize state bidirectionally, modify nupm-managed files, execute `build.nu`, or pretend that nupm’s current format is a stable package-manager API.

---

# 1. Scope

## In scope

* Read-only nupm home discovery.
* Read-only package inspection and compatibility reporting.
* Parsing supported `nupm.nuon` metadata without executing Nushell package code.
* Explicit import of compatible module packages into Numan.
* Imported-package provenance, content identity, and source-tree drift checks.
* Activation of imported modules through Phase 4’s existing managed vendor-autoload path.
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

   Imported modules use Phase 4’s `module` import mode only. `all` import mode is not available for nupm imports in Phase 6.

8. **No automatic dependency translation.**

   External dependencies declared or inferred from nupm packages remain unsupported until Numan has an explicit runtime module-linking contract.

---

# 3. Preconditions

Phase 6 requires:

```text
Phase 4:
  managed vendor-autoload activation
  module activation state and recovery
  external-file drift protection
  root mutation lock

Phase 5.1:
  lockfile v2
  BTreeMap-based deterministic serialization
  retained immutable revisions
  direct-root intent
  payload manifest identity

Phase 5 lifecycle:
  operation-scoped staging
  pending-lifecycle journal
  promote-after-validation semantics
```

Phase 6 does not require the Phase 5 plugin lifecycle capability gate.

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

## 6.1 `numan nupm status`

Read-only.

Resolution order for nupm home:

```text
1. --nupm-home PATH
2. inherited NUPM_HOME environment variable
3. no automatic guess
```

If no location is available:

```text
No nupm home was supplied.

Pass --nupm-home <path> or set NUPM_HOME for this command.
Numan will not guess or modify nupm’s installation location.
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

10. Validate the imported module entry through Phase 4’s candidate-validation path.

11. Promote to immutable Numan revision storage.

12. Write lifecycle journal progress.

13. Add the imported package as a direct root with explicit import intent.

14. Persist provenance.

15. Do not activate automatically.

16. Clear journal.

Imported revision origin:

```json
{
  "origin": "nupm_import",
  "selection_reason": "explicit_nupm_import",
  "trust_level": "local_foreign_import"
}
```

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

`diff` never changes Numan’s selected revision or nupm files.

A changed source tree requires explicit re-import.

---

# 7. Metadata Parsing

## 7.1 Parsing rules

`nupm.nuon` is metadata, not executable package code.

Phase 6 must parse supported metadata through a constrained Nuon parser or a verified isolated parser adapter.

It must never:

```text
source nupm.nuon
use nupm.nuon
overlay use nupm.nuon
execute package commands
execute build.nu
```

The parser receives only the metadata bytes and returns a normalized internal descriptor.

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

Each imported revision persists:

```json
{
  "origin": "nupm_import",
  "selection_reason": "explicit_nupm_import",
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
```

Optional Git information is descriptive only.

It does not convert a local import into registry provenance or source-build provenance.

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

It generates `use` statements with validated absolute paths into Numan’s immutable payload store. That means activation does not depend on the continuing existence of `NUPM_HOME`.

If a nupm package remains active through the user’s own environment configuration, Numan reports potential command-name overlap but does not attempt to disable or rewrite nupm state.

---

# 10. Re-import Policy

There is no live synchronization.

To adopt local nupm changes:

```text
numan nupm import <path> --as owner/name --yes
```

When the source payload hash differs:

* create a new retained immutable Numan revision;
* show the old and new payload identities;
* preserve the prior selected revision until the lifecycle transaction commits;
* require normal update-style consent before selecting the new revision;
* retain the old revision for snapshot rollback and garbage-collection rules.

Numan never writes an updated revision back into nupm.

---

# 11. Journaling and Recovery

nupm import uses the Phase 5 lifecycle journal with:

```text
operation = nupm_import
operation ID
source path
metadata hash
staging paths
target scoped IDs
prior root state
desired root state
prior selection
desired selection
planned revision IDs
```

Stages:

```text
Planned
PayloadsStaged
PayloadsPromoted
SelectionCommitted
```

Recovery rules:

* Before promotion, stale staging directories may be cleaned only when the journal confirms they belong to the interrupted operation.
* After promotion but before selection commit, promoted payloads remain inert retained revisions.
* Recovery never re-reads mutable nupm source files to reconstruct an interrupted transaction.
* Recovery uses the staged or promoted immutable payload identity recorded in the journal.
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
src/core/package.rs
src/state/lockfile.rs
src/state/lifecycle_journal.rs
src/install/transaction.rs
src/cmd/mod.rs
src/main.rs
AGENTS.md
CLAUDE.md
```

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
* No-home error behavior.
* Metadata parser accepts supported fixtures.
* Metadata parser rejects executable or malformed forms.
* Module classification succeeds only with expected entry.
* Scripts, custom packages, and build hooks are rejected.
* Scoped identity is required.
* Payload manifest hash is deterministic.
* Symlink and reparse-point escape is rejected.
* Imported revision provenance round-trips.
* No nupm state mutation occurs during status, inspect, diff, or failed import.

## Integration tests

* Import a supported local module.
* Verify imported payload is under `$NUMAN_ROOT`, not linked to nupm.
* Verify source-tree change does not alter imported revision.
* Verify explicit re-import creates a new retained revision.
* Verify a failed import preserves prior state.
* Verify interrupted import recovery.
* Verify nupm source removal after import does not break active imported module.
* Verify module activation through Numan’s vendor-autoload file.
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

# 15. Definition of Done

Phase 6 is complete when:

* Numan can inspect an explicitly supplied nupm installation without changing it.
* Numan imports only supported local nupm modules.
* Every import has explicit scoped identity and immutable payload provenance.
* Imported payloads are independent from later nupm changes.
* Imported modules can activate through existing Numan module activation.
* nupm scripts, custom builds, plugins, unknown formats, and external dependencies are rejected safely.
* No command writes to `NUPM_HOME`, nupm metadata, or Nu user configuration.
* Drift is visible through `numan nupm diff`.
* Re-import is explicit and lifecycle-journaled.
* Fixture and real-Nu acceptance tests pass on Windows, Linux, and macOS.
* Bidirectional synchronization remains explicitly deferred.
