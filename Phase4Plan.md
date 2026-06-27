\# Phase 4: Module Activation Through One Managed Nu Vendor-Autoload File



\## Objective



Implement safe activation and deactivation for installed Nushell module packages.



Phase 4 extends Numan’s existing inert install model:



\* `numan install` remains inert.

\* Module activation is explicit.

\* Numan manages exactly one external Nu file.

\* Lockfile state remains authoritative.

\* Activation validates generated Nu code before replacing the live autoload file.

\* Recovery is journaled because the transaction spans Numan-owned state and one external Nu-owned location.



This phase does not implement scripts, shell completions, package dependency resolution, source builds, mutating updates, package removal, nupm synchronization, or plugin deactivation.



\---



\# 1. Scope



\## In scope



\* Module activation through a managed Nu vendor-autoload file.

\* Module deactivation through regeneration or deletion of that file.

\* Module-specific registry metadata and locked activation metadata.

\* Vendor-autoload target discovery during `numan init --refresh`.

\* Managed-file ownership, integrity, and drift checks.

\* Candidate generation and real Nu execution validation.

\* Separate module-autoload journaling and recovery.

\* Extending `numan activate` so it can activate modules alongside existing plugins.

\* `numan deactivate` for modules only.

\* Read-only module activation inspection through `--list` and `--check`.

\* Real-Nu integration coverage for module imports and candidate execution.



\## Explicitly out of scope



\* Script activation or wrapper generation.

\* `$env.PATH` manipulation.

\* `$NU\_LIB\_DIRS` manipulation.

\* Fish, Bash, Zsh, PowerShell, or other non-Nu completion installation.

\* Plugin deactivation.

\* Automatic dependency installation or ordering.

\* Module activation where registry metadata declares external Numan package dependencies.

\* Source builds.

\* Update, remove, rollback, cleanup, or nupm synchronization.

\* Automatic migration between different Nu data directories or vendor-autoload targets.



\---



\# 2. Architectural Rules



1\. \*\*Install remains inert.\*\* Module installation writes only under `$NUMAN\_ROOT`.



2\. \*\*Numan owns one external file only.\*\*



&#x20;  ```text

&#x20;  <cached selected vendor-autoload directory>/numan.nu

&#x20;  ```



&#x20;  Numan may create, replace, or delete only that file.



3\. \*\*Never modify user configuration.\*\*



&#x20;  Do not modify:



&#x20;  ```text

&#x20;  config.nu

&#x20;  env.nu

&#x20;  user autoload files

&#x20;  other vendor autoload files

&#x20;  user module directories

&#x20;  $NU\_LIB\_DIRS

&#x20;  $env.PATH

&#x20;  ```



4\. \*\*Lockfile membership is authoritative.\*\*



&#x20;  A module is active only when its lockfile entry has a current module activation record. `autoload-state.json` is a derived verification projection, not a second authority.



5\. \*\*Autoload state is a controlled external mutation.\*\*



&#x20;  Module activation is not read-only. It changes the future startup behavior of Nushell.



6\. \*\*No global transaction claim.\*\*



&#x20;  Numan cannot atomically commit both a Nu vendor file and internal JSON state. It must use a journal to make interrupted work detectable and safely recoverable.



7\. \*\*Plugin and module activation are separate transactions.\*\*



&#x20;  A single `numan activate` invocation may process both types, but plugin registration and module-autoload replacement remain sequential and independently journaled.



\---



\# 3. Phase 3 Compatibility



Phase 3’s existing `activation` field remains the plugin activation record. Do not rename or reinterpret it in Phase 4.



Add module-specific state rather than converting the existing plugin field into a broad enum. This preserves compatibility with current lockfiles and keeps plugin registration semantics distinct from vendor-autoload semantics.



Current plugin activation state is tied to:



```text

Nu executable SHA-256

Nu version

Nu plugin registry path

```



Module activation instead needs to be tied to:



```text

Nu executable SHA-256

Nu version

selected vendor-autoload directory

managed numan.nu path

```



Phase 4 adds:



```text

src/state/autoload\_state.rs

src/state/autoload\_journal.rs

src/nu/autoload.rs

src/cmd/deactivate.rs

```



Phase 4 may extend shared atomic-write and filesystem-safety utilities, but must not change Phase 3 plugin behavior except where needed to support combined command planning and a shared mutation lock.



\---



\# 4. Registry and Locked Metadata



\## 4.1 Registry module activation metadata



Add module activation metadata to `VersionEntry`.



```rust

pub struct VersionEntry {

&#x20;   pub version: Version,

&#x20;   pub nu\_version: String,

&#x20;   pub verified\_with: Vec<String>,

&#x20;   pub artifact: Artifact,

&#x20;   pub source: Option<SourceInfo>,

&#x20;   pub dependencies: BTreeMap<String, String>,

&#x20;   pub activation: Option<RegistryActivationSpec>,

}

```



```rust

pub struct RegistryActivationSpec {

&#x20;   pub kind: String,

&#x20;   #\[serde(default)]

&#x20;   pub import: ModuleImportMode,

}

```



```rust

pub enum ModuleImportMode {

&#x20;   Module,

&#x20;   All,

}

```



Registry contract for modules:



```json

{

&#x20; "version": "1.0.0",

&#x20; "nu\_version": ">=0.113.0",

&#x20; "artifact": {

&#x20;   "kind": "archive",

&#x20;   "url": "https://example.invalid/foo.zip",

&#x20;   "sha256": "...",

&#x20;   "entry": "mod.nu"

&#x20; },

&#x20; "activation": {

&#x20;   "kind": "nu-module",

&#x20;   "import": "module"

&#x20; },

&#x20; "dependencies": {}

}

```



Rules:



\* `artifact.entry` is required for module activation.

\* `artifact.entry` must be a normalized relative `.nu` path.

\* `activation.kind` must be `nu-module` for a module package.

\* Omitted `activation.import` defaults to `module`.

\* `all` must be explicitly declared.

\* Unknown activation kinds are rejected.

\* Module packages with non-empty registry `dependencies` are installable but not activatable in Phase 4.

\* `source` is irrelevant to dependency eligibility.



\## 4.2 Persist activation-relevant metadata at install time



Activation must not re-query a registry to discover the metadata for an already-installed package.



Extend `LockfileEntry` so installation persists:



```rust

pub struct LockfileEntry {

&#x20;   // Existing fields...



&#x20;   #\[serde(default)]

&#x20;   pub locked\_dependencies: BTreeMap<String, String>,



&#x20;   #\[serde(default)]

&#x20;   pub module\_import\_mode: Option<ModuleImportMode>,



&#x20;   #\[serde(default)]

&#x20;   pub module\_activation: Option<ModuleActivation>,

}

```



The install transaction must populate:



```text

locked\_dependencies

module\_import\_mode

entry

payload\_path

artifact SHA-256

registry provenance

```



For Phase 4 activation:



```text

locked\_dependencies must be empty

entry must exist

module\_import\_mode must be known

```



Do not inspect `source` to infer dependencies.



\## 4.3 Module activation record



```rust

pub struct ModuleActivation {

&#x20;   pub entry\_path: String,

&#x20;   pub import\_mode: ModuleImportMode,



&#x20;   pub vendor\_autoload\_dir: String,

&#x20;   pub managed\_file\_path: String,



&#x20;   pub nu\_executable\_sha256: String,

&#x20;   pub nu\_version: String,



&#x20;   pub activated\_at: String,

}

```



A module is current-active only when:



\* `module\_activation` exists;

\* its Nu executable hash matches cached `NuPaths`;

\* its Nu version matches cached `NuPaths`;

\* its selected vendor-autoload directory matches cached `NuPaths`;

\* its managed file path matches the selected `<vendor-dir>/numan.nu`;

\* the central autoload state matches lockfile membership;

\* the managed file passes ownership and SHA-256 verification.



\---



\# 5. NuPaths and Vendor-Autoload Target Discovery



\## 5.1 Extend the Nu probe



Extend `NuPaths` so `numan init` and `numan init --refresh` cache:



```rust

pub struct NuPaths {

&#x20;   pub nu\_executable: String,

&#x20;   pub nu\_version: String,

&#x20;   pub plugin\_registry\_path: String,

&#x20;   pub nu\_executable\_hash: String,

&#x20;   pub platform: String,



&#x20;   pub data\_dir: Option<String>,

&#x20;   pub vendor\_autoload\_dirs: Vec<String>,

&#x20;   pub vendor\_autoload\_dir: Option<String>,

}

```



Replace the current line-oriented Nu probe with one static program that emits one machine-readable JSON object containing:



```text

Nu version

Nu plugin registry path

Nu data directory

Nu vendor-autoload directories

```



Parse output through `serde\_json`. Do not parse path lists from ad hoc line splitting.



\## 5.2 Safe target-selection policy



During `numan init --refresh`:



1\. Discover and hash the absolute Nu executable.



2\. Probe Nu for `$nu.data-dir` and `$nu.vendor-autoload-dirs`.



3\. Compute the expected Numan-safe target:



&#x20;  ```text

&#x20;  <$nu.data-dir>/vendor/autoload

&#x20;  ```



4\. Normalize both the expected target and reported vendor-autoload paths.



5\. Select the expected target only when it is present in Nu’s reported vendor-autoload list.



6\. Cache the selected directory in `NuPaths.vendor\_autoload\_dir`.



If the expected target is absent:



```text

No Numan-safe vendor-autoload directory is available.



Numan requires <Nu data-dir>/vendor/autoload to be present in

$nu.vendor-autoload-dirs. Run `numan init --refresh` after fixing the

Nushell installation or configuration.

```



Do not choose the first writable vendor directory. It may belong to the system, a distribution package, or another tool.



\## 5.3 Refresh behavior with active modules



If an existing autoload state file reports active modules:



\* Validate the managed file’s ownership marker, SHA-256, and lockfile projection.

\* Revalidate the current managed autoload file using the newly detected Nu executable.

\* If validation succeeds and the selected vendor directory is unchanged, update module activation identity fields and autoload-state metadata atomically.

\* If validation fails, or if the selected vendor directory changed, do not silently migrate or overwrite anything.



For target changes, return a clear error. Phase 4 does not automatically migrate an active managed autoload file to a new Nu data directory.



\---



\# 6. Managed File Ownership and Safety



\## 6.1 Managed file



The only external file owned by Numan is:



```text

<cached vendor-autoload directory>/numan.nu

```



Every generated file begins exactly with:



```nu

\# Generated and managed by Numan. Do not edit.

\# Numan autoload schema: 1

```



Use plain UTF-8 without BOM.



\## 6.2 Ownership checks before overwrite or delete



Before replacing or deleting `numan.nu`, require all of the following:



1\. The vendor-autoload directory is not a symlink or Windows reparse point.

2\. The managed file is not a symlink or Windows reparse point.

3\. The file begins with the exact ownership marker.

4\. The on-disk SHA-256 equals `autoload-state.json.generated\_file\_sha256`.

5\. `autoload-state.json.active\_module\_ids` equals the active module set derived from lockfile records.

6\. The stored vendor directory and managed-file path equal the current cached target.



If the file exists but no matching Numan state exists, do not take ownership of it.



If any condition fails:



```text

Numan managed-file drift detected.



numan.nu was changed, replaced, moved, or is no longer a Numan-owned regular

file. Numan will not overwrite or delete it automatically.

```



\## 6.3 Filesystem containment rules



For every module entry:



\* `payload\_path` must be relative.

\* `payload\_path` must not contain `..`, a root component, or a platform prefix.

\* `entry\_path` must be relative.

\* `entry\_path` must not contain `..`, a root component, or a platform prefix.

\* Canonical payload path must remain under canonical `$NUMAN\_ROOT`.

\* Canonical entry path must remain under canonical payload directory.

\* Entry must be a regular file.

\* Entry extension must be `.nu`.



Do not follow symlinks that escape the payload directory.



\---



\# 7. Autoload State



Create:



```text

$NUMAN\_ROOT/nu\_state/autoload-state.json

```



```json

{

&#x20; "schema\_version": 1,

&#x20; "vendor\_autoload\_dir": "C:/Users/example/AppData/Local/nushell/vendor/autoload",

&#x20; "managed\_file\_path": "C:/Users/example/AppData/Local/nushell/vendor/autoload/numan.nu",

&#x20; "nu\_executable\_sha256": "…",

&#x20; "nu\_version": "…",

&#x20; "generated\_file\_sha256": "…",

&#x20; "active\_module\_ids": \[

&#x20;   "owner/foo",

&#x20;   "owner/bar"

&#x20; ],

&#x20; "generated\_at": "…"

}

```



Rules:



\* Lockfile module activation records are authoritative.

\* `active\_module\_ids` is a deterministic, sorted projection.

\* `autoload-state.json` is written only after the managed file is successfully replaced or removed.

\* A disagreement blocks activation and deactivation until recovery completes.

\* When no modules remain active:



&#x20; \* delete Numan’s managed `numan.nu`;

&#x20; \* delete `autoload-state.json`;

&#x20; \* clear module activation records from lockfile.



\---



\# 8. Generated Autoload Content



\## 8.1 Deterministic ordering



Generate statements by ascending canonical scoped package ID.



```text

owner/alpha

owner/beta

owner/zeta

```



Do not preserve installation order.



\## 8.2 Import rendering



Use one function only:



```rust

render\_use\_statement(path: \&Path, mode: ModuleImportMode) -> Result<String>

```



Its output contract:



```text

Module -> use <Nu-escaped absolute module path>

All    -> use <Nu-escaped absolute module path> \*

```



Do not render paths through shell quoting.



Do not interpolate paths into `nu -c` command strings.



The renderer must produce a valid Nu module-path literal for:



\* Windows paths;

\* Unix paths;

\* spaces;

\* backslashes;

\* double quotes;

\* apostrophes;

\* Unicode;

\* paths containing parentheses, brackets, or other punctuation.



Generated example:



```nu

\# Generated and managed by Numan. Do not edit.

\# Numan autoload schema: 1



use "A:\\\\numan\\\\packages\\\\modules\\\\owner\\\\foo\\\\1.0.0-a1b2c3d4\\\\mod.nu"

use "A:\\\\numan\\\\packages\\\\modules\\\\owner\\\\bar\\\\1.2.0-d4c3b2a1\\\\mod.nu" \*

```



The exact renderer syntax is accepted only after real-Nu validation on Windows, Linux, and macOS.



\---



\# 9. Candidate Generation and Nu Validation



\## 9.1 Candidate location



Never create temporary `.nu` files in a vendor-autoload directory. Nu could load them during another shell startup.



Create a same-directory temporary candidate with a non-`.nu` suffix:



```text

.<uuid>.candidate.tmp

```



Example:



```text

C:\\Users\\example\\AppData\\Local\\nushell\\vendor\\autoload\\.a1b2c3.candidate.tmp

```



Use a same-directory temporary file so replacement remains on the same filesystem.



\## 9.2 Candidate validation



Validate a generated candidate by executing it directly with the cached absolute Nu executable:



```text

<cached-nu-executable> -n <candidate-path>

```



Requirements:



\* Use `std::process::Command`.

\* Pass candidate path as an argument, never inside a command string.

\* Capture stdout and stderr.

\* Treat nonzero exit status as validation failure.

\* Include package ID and Nu stderr in the error summary.

\* Remove the candidate on validation failure.

\* Preserve existing managed file and all lockfile state on validation failure.



Validation must prove:



\* generated syntax is valid;

\* `use` statements resolve;

\* internal imports within the module payload resolve;

\* collisions or import-time errors reported by Nu fail the candidate.



Phase 4 must add real-Nu integration tests proving that:



```text

nu -n <non-.nu candidate file>

```



works on Windows, Linux, and macOS.



\---



\# 10. Module-Autoload Journal



Create a separate journal:



```text

$NUMAN\_ROOT/state/pending-autoload.json

```



Do not reuse the plugin-only pending activation journal.



```rust

pub struct PendingAutoload {

&#x20;   pub schema\_version: u32,

&#x20;   pub operation: AutoloadOperation,

&#x20;   pub stage: AutoloadStage,



&#x20;   pub nu\_executable\_sha256: String,

&#x20;   pub nu\_version: String,

&#x20;   pub vendor\_autoload\_dir: String,

&#x20;   pub managed\_file\_path: String,



&#x20;   pub previous\_file\_exists: bool,

&#x20;   pub previous\_file\_sha256: Option<String>,



&#x20;   pub desired\_file\_exists: bool,

&#x20;   pub candidate\_sha256: Option<String>,

&#x20;   pub previous\_active\_module\_ids: Vec<String>,

&#x20;   pub desired\_active\_module\_ids: Vec<String>,



&#x20;   pub targeted\_module\_ids: Vec<String>,

&#x20;   pub created\_at: String

}

```



```rust

pub enum AutoloadOperation {

&#x20;   Activate,

&#x20;   Deactivate,

&#x20;   RevalidateAfterRefresh,

}

```



```rust

pub enum AutoloadStage {

&#x20;   Prepared,

&#x20;   Replaced,

}

```



\## 10.1 Transaction protocol



For non-empty desired module sets:



1\. Acquire the Numan root mutation lock.

2\. Validate current Nu identity and vendor target.

3\. Verify managed-file ownership and state projection.

4\. Generate candidate.

5\. Execute candidate with cached Nu.

6\. Snapshot lockfile.

7\. Atomically write journal at `Prepared`.

8\. Atomically replace `numan.nu` with the validated candidate.

9\. Atomically write journal at `Replaced`.

10\. Atomically write lockfile module activation records.

11\. Atomically write derived `autoload-state.json`.

12\. Clear the journal.

13\. Release mutation lock.



For final deactivation, where desired module membership is empty:



1\. Verify ownership, hash, and state projection.

2\. Snapshot lockfile.

3\. Write `Prepared` journal with `desired\_file\_exists = false`.

4\. Delete only the verified Numan-managed `numan.nu`.

5\. Write `Replaced` journal.

6\. Clear module activation records from lockfile.

7\. Delete `autoload-state.json`.

8\. Clear journal.



\## 10.2 Failure rules



Before managed-file replacement:



```text

No external state changes.

```



After managed-file replacement:



```text

Do not attempt silent rollback.

Do not clear the journal.

Do not claim the transaction completed.

```



If lockfile or autoload-state persistence fails after replacement, retain the `Replaced` journal for recovery.



\## 10.3 Recovery



Before activation or deactivation, inspect `pending-autoload.json`.



\### Prepared journal



Verify live managed-file state still matches the recorded previous state.



If it does:



\* remove the leftover candidate if present;

\* clear the journal;

\* report that no external replacement occurred.



If it does not:



\* block mutation;

\* report managed-file drift.



\### Replaced journal



Verify:



\* current Nu identity matches journal identity;

\* managed file ownership and SHA-256 match journal candidate SHA;

\* desired active module IDs are valid lockfile package IDs.



If verification succeeds:



\* finish lockfile activation updates;

\* write derived autoload state;

\* clear journal;

\* print recovery status.



If verification fails:



\* block further mutation;

\* preserve the journal;

\* report the exact mismatch.



\---



\# 11. Mutation Serialization



Atomic file replacement prevents partial writes. It does not prevent two Numan processes from overwriting each other’s lockfile or managed autoload projection.



Add one root-scoped exclusive mutation lock:



```text

$NUMAN\_ROOT/state/mutation.lock

```



Use an advisory filesystem lock held for the entire plugin or module mutation transaction.



Commands requiring the lock:



```text

numan install

numan activate

numan deactivate

numan init --refresh when active autoload state exists

```



Read-only commands do not require it:



```text

numan activate --list

numan activate --check

numan list

numan info

```



If the lock is unavailable, return a clear error rather than racing:



```text

Another Numan mutation is already in progress for this root.

Wait for it to finish, then retry.

```



\---



\# 12. Command Surface



\## 12.1 Activate



```text

numan activate \[PACKAGE...]

numan activate --list

numan activate --check \[PACKAGE...]

```



\### `numan activate \[PACKAGE...]`



Behavior:



\* Explicit package IDs may include plugins or modules.

\* Plugin targets use the existing Phase 3 registration lane.

\* Module targets use the Phase 4 managed-autoload lane.

\* Script and completion package IDs fail with a deferred-feature error.

\* Unknown package IDs fail before mutation.

\* Explicit package IDs that are already current-active are listed as skipped.

\* No package IDs means:



&#x20; \* activate all inactive plugins;

&#x20; \* activate all inactive modules;

&#x20; \* skip packages that are already active for current identity.



Show one consent table grouped by operation:



```text

Plugin registration

&#x20; owner/plugin-a -> <cached plugin registry>



Module startup autoload

&#x20; owner/module-a -> <managed vendor file>

&#x20; owner/module-b -> <managed vendor file>

```



Require interactive confirmation unless `--yes` is supplied.



For non-TTY sessions without `--yes`, fail before mutation.



Execution order:



1\. Plugin lane.

2\. Module lane.

3\. Combined summary.



Do not process mutation lanes in parallel.



A failure in one lane does not undo a completed transaction in the other lane. Return nonzero if any requested activation failed.



\### `numan activate --list`



Read-only output.



Show:



```text

package ID

type

status

version

autoload target or plugin registry target

last validated Nu version

```



Module statuses include:



```text

active

inactive

managed-file drift

state projection mismatch

pending recovery

stale Nu validation

```



\### `numan activate --check \[PACKAGE...]`



Read-only integrity check.



For modules, verify:



\* NuPaths cache is usable;

\* managed target selection is still valid;

\* lockfile and autoload-state projection match;

\* managed-file ownership marker and hash;

\* module payload and entry path containment;

\* module entry is a regular `.nu` file;

\* module has no locked external Numan dependencies.



Do not replace managed files.

Do not alter lockfile state.

Do not run plugin registration.



\---



\## 12.2 Deactivate



```text

numan deactivate \[PACKAGE...]

```



Behavior:



\* Supports modules only in Phase 4.



\* Plugin package IDs return:



&#x20; ```text

&#x20; Plugin deactivation is deferred to a later phase.

&#x20; ```



\* Script and completion package IDs return a deferred-feature error.



\* Explicit module package IDs must be currently active.



\* No package IDs means deactivate all current active modules after confirmation.



\* Require interactive confirmation unless `--yes` is supplied.



For partial module deactivation:



\* regenerate and validate a candidate containing remaining active modules;

\* execute the normal module-autoload transaction.



For full module deactivation:



\* verify ownership and hash;

\* delete only Numan-managed `numan.nu`;

\* clear module activation records;

\* remove autoload-state;

\* use the journal protocol.



\---



\# 13. Nu Drift and Refresh Policy



Plugin and module activation cannot share identical drift semantics.



\## Plugins



Plugin activation remains tied to:



```text

Nu executable hash

Nu version

plugin registry path

```



\## Modules



Module activation remains tied to:



```text

Nu executable hash

Nu version

vendor-autoload target

managed autoload file path

```



When Nu binary drift is detected:



\* `numan activate` and partial `numan deactivate` refuse until `numan init --refresh`.

\* `numan init --refresh` re-probes Nu and validates existing managed module state.

\* If existing managed module imports validate successfully with the new Nu and the target did not change, refresh updates module activation identity records atomically.

\* If validation fails or target changed, Numan does not silently move or overwrite the managed file.



Full deactivation of all modules may proceed under stale Nu identity because it removes only a verified Numan-managed file and does not execute a candidate.



\---



\# 14. Files



\## New files



```text

src/nu/autoload.rs

src/state/autoload\_state.rs

src/state/autoload\_journal.rs

src/cmd/deactivate.rs

src/util/fs\_safety.rs

tests/module\_autoload\_test.rs

```



\## Modified files



```text

src/core/package.rs

src/install/transaction.rs

src/state/lockfile.rs

src/state/mod.rs

src/nu/paths.rs

src/nu/mod.rs

src/cmd/activate.rs

src/cmd/mod.rs

src/main.rs

src/util/atomic.rs

AGENTS.md

CLAUDE.md

Cargo.toml

```



\## Responsibilities



\### `src/core/package.rs`



\* Add registry activation metadata.

\* Add `ModuleImportMode`.

\* Preserve `dependencies` as registry metadata.



\### `src/install/transaction.rs`



\* Persist module entry, import mode, and locked dependencies.

\* Continue inert installation behavior.



\### `src/state/lockfile.rs`



\* Preserve existing plugin `activation`.

\* Add `module\_activation`.

\* Add `locked\_dependencies`.

\* Add module active-state helpers.

\* Bump lockfile schema version with backward-compatible defaults.



\### `src/nu/paths.rs`



\* Probe and cache Nu data directory and vendor-autoload directories.

\* Select only the safe data-directory vendor target.

\* Add module-autoload environment drift validation.



\### `src/nu/autoload.rs`



\* Resolve validated module entries.

\* Render deterministic autoload content.

\* Create and validate candidates.

\* Replace or delete the managed file.

\* Provide injected candidate runner for tests.



\### `src/state/autoload\_state.rs`



\* Load, save, validate, and compare derived state projection.



\### `src/state/autoload\_journal.rs`



\* Persist `Prepared` and `Replaced` module-autoload transaction stages.

\* Reconcile interrupted operations.



\### `src/util/fs\_safety.rs`



\* Detect symlinks and Windows reparse points.

\* Validate root containment.

\* Validate regular files.

\* Guard managed external file operations.



\### `src/cmd/activate.rs`



\* Split planning into plugin and module lanes.

\* Preserve existing plugin registrar path.

\* Add module target resolution, consent grouping, and combined summary.



\### `src/cmd/deactivate.rs`



\* Implement module-only deactivation.

\* Reject plugin deactivation clearly.



\---



\# 15. Implementation Order



\## Step 1: Schema and compatibility



\* Add registry module activation metadata.

\* Add module import mode enum.

\* Add lockfile module fields.

\* Persist locked module metadata at install.

\* Add backward-compatible serde defaults.

\* Add schema migration tests.



\## Step 2: Nu environment caching



\* Extend Nu probe to emit structured data.

\* Cache data directory and vendor-autoload directory list.

\* Implement safe vendor target selection.

\* Add refresh and drift tests.



\## Step 3: Shared filesystem and mutation safety



\* Add root mutation lock.

\* Add symlink/reparse-point detection.

\* Add root containment helpers.

\* Add arbitrary-byte atomic replacement support.



\## Step 4: Module state and journal



\* Implement autoload-state projection.

\* Implement autoload journal.

\* Implement prepared/replaced recovery paths.



\## Step 5: Candidate generation



\* Implement validated entry resolution.

\* Implement `render\_use\_statement`.

\* Implement deterministic file generation.

\* Implement real Nu candidate runner seam.



\## Step 6: Module activation



\* Extend activation planner.

\* Implement module consent display.

\* Implement candidate validation and replacement transaction.

\* Preserve independent plugin lane behavior.



\## Step 7: Module deactivation



\* Implement partial and full module deactivation.

\* Implement final managed-file deletion.

\* Implement stale-Nu full-deactivation path.



\## Step 8: Documentation and acceptance



\* Update AGENTS.md and CLAUDE.md.

\* Run automated tests.

\* Run real-Nu acceptance tests on Windows, Linux, and macOS.



\---



\# 16. Test Plan



\## Unit tests



\### Registry and install metadata



\* Module registry metadata defaults import mode to `module`.

\* `all` is accepted only when explicitly declared.

\* Unknown activation kind fails.

\* Module entry must be relative `.nu`.

\* Locked dependencies and import mode survive lockfile round-trip.



\### NuPaths target selection



\* Safe data-directory vendor path present in list is selected.

\* Path absent from vendor list fails.

\* Equivalent normalized paths match.

\* Different vendor directory list is drift.

\* Existing old lockfiles require `init --refresh` before module operations.



\### File safety



\* Reject `..`, absolute paths, prefixes, and path traversal.

\* Reject symlink escape from payload.

\* Reject symlink/reparse managed directory.

\* Reject symlink/reparse managed file.

\* Reject non-regular module entry.

\* Reject pre-existing unmanaged `numan.nu`.



\### Rendering and generation



\* Stable ordering by scoped ID.

\* Empty projection behavior.

\* Windows path escaping.

\* Unix path escaping.

\* Spaces, quotes, Unicode, and backslashes.

\* `module` and `all` rendering.



\### State and journal



\* Lockfile remains authoritative.

\* Autoload-state mismatch blocks mutation.

\* Ownership marker mismatch blocks mutation.

\* Managed file SHA mismatch blocks mutation.

\* Prepared recovery clears only when prior state matches.

\* Replaced recovery finalizes only when candidate state matches.

\* Stale or mismatched journal blocks mutation.



\### Command planning



\* Explicit plugin and module IDs are classified correctly.

\* Scripts and completions fail as deferred.

\* Already active modules are skipped.

\* Non-TTY activation without `--yes` fails.

\* `--list` and `--check` are mutation-free.

\* Plugin deactivation returns deferred-feature error.



\## Integration tests with injected runner



\* Module activation writes expected candidate and state.

\* Module validation failure preserves prior managed file.

\* Partial deactivation keeps remaining module import.

\* Full deactivation deletes only managed file.

\* Lockfile save failure after replacement leaves `Replaced` journal.

\* Recovery completes lockfile and autoload-state updates.

\* Plugin lane success plus module lane failure returns nonzero while preserving plugin success.

\* Concurrent mutation lock prevents two writers.



\## Real-Nu integration tests



These may be `#\[ignore]` in ordinary unit runs, but must run in a platform acceptance job or manual release checklist.



\* `nu -n <non-.nu candidate>` executes successfully.

\* Generated autoload file loads a module in a fresh normal Nu session.

\* Module commands are available after startup.

\* Import mode `module` remains namespaced.

\* Import mode `all` exposes exported commands.

\* Windows path with spaces works.

\* Windows path containing quotes or Unicode works.

\* Linux and macOS equivalent path cases work.

\* Syntax or internal import failure preserves old managed file.

\* Re-running activation is idempotent.

\* Full deactivation removes module availability in a fresh Nu session.



\---



\# 17. Definition of Done



Phase 4 is complete only when all of the following are true:



\* Module installation remains inert.

\* Module activation uses only one Numan-managed vendor-autoload file.

\* Numan never edits Nu user configuration.

\* Vendor-autoload target is cached through `numan init --refresh`.

\* Lockfile is authoritative for active module membership.

\* Autoload state is derived and integrity-checked.

\* Generated module imports are deterministic and validated by real Nu execution.

\* Manual edits, symlinks, reparse points, and state drift block mutation.

\* Module activation and deactivation are journaled.

\* Plugin and module activation run as separate sequential transactions.

\* Full module deactivation deletes only the verified Numan-managed file.

\* Existing plugin activation tests still pass.

\* New module unit and integration tests pass.

\* Real-Nu acceptance succeeds on Windows, Linux, and macOS.

\* Scripts, completions, update, remove, dependency resolution, plugin deactivation, and nupm work remain deferred.



\# 18. Deferred Work



\## Phase 5 or later



\* Script command wrappers and argument forwarding.

\* Nu-native completion modules.

\* Plugin deactivation.

\* Module dependency resolution and ordered activation.

\* Vendor-target migration.

\* Package update and version retention.

\* Package removal and cleanup.

\* Snapshot rollback.

\* Source builds.

\* nupm conflict detection and interop.

\* Doctor and repair workflows beyond journal recovery.



