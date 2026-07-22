# Active plugin mutation gate (Issue #22)

## Safety invariant

While a package has a lockfile `activation` record (plugin activation), Numan must not **remove** that package until the activation record is cleared. Plugin deactivate clears the record via a journaled `plugin rm` flow. `--force` on `numan remove` does **not** bypass active plugin activation. It only bypasses active *module* activation (`module_activation`).

**Update** of an active plugin is orchestrated when the mutation flag is enabled (default **on**): deactivate → upgrade → reactivate. Set `NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION=0` (or `false` / `FALSE` / `no`) to refuse active updates and require manual deactivate first.

## Current behavior

| Operation | Active plugin | Active module |
|---|---|---|
| `numan remove` | **Always refused** while `activation` is set (even when mutation enabled) | Refused unless `--force` |
| `numan update` | Orchestrated deactivate→upgrade→activate when enabled; refused when kill switch off | Refused (use `deactivate` first) |
| `numan deactivate` | Supported: journaled unregister + clear `activation` (payload kept) | Supported today |

After `numan deactivate <pkg>`, `numan remove <pkg>` succeeds without `--force` (inactive plugin).

### Env kill switch

```text
NUMAN_ENABLE_ACTIVE_PLUGIN_MUTATION=0   # disable active update orchestration
```

Unset or any other value leaves orchestration **enabled** (default).

### Lifecycle journal (active update)

`state/pending-lifecycle.json` with `op: update` reuses existing stages:

- `prepared`: before/during deactivate (also `pending-plugin-deactivate.json`)
- `lockfile_updated`: after install upgrade; reactivate may still be pending (`pending-activation.json`)

Cleared only after a successful full path (including reactivate when orchestration ran).

### Reporting

- `numan activate --list`: for packages with plugin activation, prints whether remove stays gated and whether update is permitted under the current kill switch.
- `numan doctor`: info finding `activation.plugin_mutation_gated` (deactivate available; remove still gated; update when enabled). Pending deactivate journal: `journal.plugin_deactivate_pending` (warn); `--fix` runs deactivate reconcile.

Canonical remove hint: `util::hints::active_plugin_mutation_gated`. Update kill-switch hint: `util::hints::active_plugin_update_disabled`.

## Shared helpers

`cmd::plugin_lifecycle` exposes `deactivate_one_plugin` / `activate_one_plugin` for lock-holding callers (notably `update`). Full CLI consent/lock/snapshot stay in `deactivate` / `activate`.

## Evidence matrix (Issue #22)

| Scenario | Evidence |
|---|---|
| Remove while active (incl. `--force`) | Unit tests (`cmd::remove`); Stage 1 asserts activation still present after list |
| Deactivate → remove → gc | [Stage 1 acceptance](acceptance/official-registry-stage1.md) on Linux/macOS/Windows CI |
| Active update orchestration | Unit tests with injectable unregister/register/install hooks (`cmd::update`) |
| Fault injection (unregister failure) | Unit tests leave `activation` + deactivate/lifecycle journals |
| Ownership / name targeting | Unregister uses lockfile `executable_path` → plugin name only (no path-similarity guesses) |
| Real-Nu smoke marker | `tests/plugin_lifecycle_real_nu.rs` (ignored; points at Stage 1) |

See also: [docs/acceptance/official-registry-stage1.md](acceptance/official-registry-stage1.md).
