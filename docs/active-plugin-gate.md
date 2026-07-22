# Active plugin mutation gate (Issue #22)

## Safety invariant

While a package has a lockfile `activation` record (plugin activation), Numan must not mutate that package via remove or update until the activation record is cleared. Plugin deactivate clears the record via a journaled `plugin rm` flow. `--force` on `numan remove` does **not** bypass active plugin activation. It only bypasses active *module* activation (`module_activation`).

## Current behavior

| Operation | Active plugin | Active module |
|---|---|---|
| `numan remove` | Refused while `activation` is set (Issue #22 hint) | Refused unless `--force` |
| `numan update` | Refused (Issue #22 hint) | Refused (use `deactivate` first) |
| `numan deactivate` | Supported: journaled unregister + clear `activation` (payload kept) | Supported today |

After `numan deactivate <pkg>`, `numan remove <pkg>` succeeds without `--force` (inactive plugin).

`numan doctor` reports an **info** finding `activation.plugin_mutation_gated` for each package with `package_type == "plugin"` and `activation.is_some()` (even when `nu_state/paths.json` is missing). A pending deactivate journal surfaces as `journal.plugin_deactivate_pending` (warn); `--fix` runs deactivate reconcile.

Canonical hint text lives in `util::hints::active_plugin_mutation_gated` /
`ACTIVE_PLUGIN_MUTATION_GATED_FIX` (remove: deactivate then remove) and
`util::hints::active_plugin_update_gated` (update: deactivate then update).

## Deferred (Issue #22 remainder)

- Full safety matrix from [Issue #22](https://github.com/tonythethompson/numan/issues/22) (fault injection across every phase, real-Nu multi-OS coverage, active-plugin update)
- Active-plugin update remains gated

See also: [docs/acceptance/official-registry-stage1.md](acceptance/official-registry-stage1.md).
