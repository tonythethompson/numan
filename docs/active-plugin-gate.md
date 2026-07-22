# Active plugin mutation gate (Issue #22)

## Safety invariant

While a package has a lockfile `activation` record (plugin activation), Numan must not **remove** that package until the activation record is cleared. Plugin deactivate clears the record via a journaled `plugin rm` flow. `--force` on `numan remove` does **not** bypass active plugin activation. It only bypasses active *module* activation (`module_activation`).

**Update** of an active plugin is orchestrated only when the mutation flag is **enabled** (default **off**, opt-in until the Issue #22 evidence matrix is green): deactivate → upgrade → reactivate. Set `NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION=1` (or `true` / `TRUE` / `yes`) to allow active updates. Otherwise update refuses while a matching activation is set and requires manual deactivate first.

## Current behavior

| Operation | Active plugin | Active module |
|---|---|---|
| `numan remove` | **Always refused** while `activation` is set (even when mutation enabled) | Refused unless `--force` |
| `numan update` | Orchestrated deactivate→upgrade→activate when opt-in enabled; refused when disabled (default) | Refused (use `deactivate` first) |
| `numan deactivate` | Supported: journaled unregister + clear `activation` (payload kept) | Supported today |

After `numan deactivate <pkg>`, `numan remove <pkg>` succeeds without `--force` (inactive plugin).

### Env enable switch (opt-in)

```text
NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION=1   # enable active update orchestration
```

Unset or any value other than `1` / `true` / `TRUE` / `yes` keeps orchestration **disabled** (default).

### Lifecycle journal (active update)

`state/pending-lifecycle.json` with `op: update` reuses existing stages:

- `prepared`: before/during deactivate (also `pending-plugin-deactivate.json`)
- `lockfile_updated`: after install upgrade; reactivate may still be pending (`pending-activation.json`)

Cleared only after a successful full path (including reactivate when orchestration ran).

### Reporting

- `numan activate --list`: for packages with plugin activation, prints whether remove stays gated and whether update is permitted under the current enable switch.
- `numan doctor`: info finding `activation.plugin_mutation_gated` (deactivate available; remove still gated; update opt-in). Pending deactivate journal: `journal.plugin_deactivate_pending` (warn); `--fix` runs deactivate reconcile.

Canonical remove hint: `util::hints::active_plugin_mutation_gated`. Update disabled hint: `util::hints::active_plugin_update_disabled`.

## Shared helpers

`cmd::plugin_lifecycle` exposes `deactivate_one_plugin` / `activate_one_plugin` for lock-holding callers (notably `update`). Full CLI consent/lock/snapshot stay in `deactivate` / `activate`.

## Evidence matrix (Issue #22)

| Scenario | Evidence |
|---|---|
| Remove while active (incl. `--force`) | Unit tests (`cmd::remove`); Stage 1 asserts activation still present after list |
| Deactivate → remove → gc | [Stage 1 acceptance](acceptance/official-registry-stage1.md) on Linux/macOS/Windows CI |
| Active update orchestration | Unit tests with injectable unregister/register/install hooks (`cmd::update`); **not** yet a real-Nu multi-OS e2e |
| Fault injection (unregister failure / failed upgrade restore) | Unit tests leave journals / attempt reactivation; full fault-injection matrix still required before default-on |
| Ownership / name targeting | Lockfile `is_active_for` + binary file existence + unregister name from `executable_path` (no msgpackz parse) |
| Real-Nu smoke marker | `tests/plugin_lifecycle_real_nu.rs` (ignored; lists required scenarios as TODO) |

**Stage 1 covers deactivate→remove on 3 OS.** Active **update** real-Nu e2e and the full fault-injection matrix remain **required before flipping default on**. Unit tests cover fake-hook orchestration only.

See also: [docs/acceptance/official-registry-stage1.md](acceptance/official-registry-stage1.md).
