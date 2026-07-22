# Active plugin mutation gate (Issue #22)

## Safety invariant

While a package has a lockfile `activation` record (plugin activation), Numan must not mutate that package via remove, update, or deactivate until the Issue #22 safety matrix is green.

`--force` on `numan remove` does **not** bypass active plugin activation. It only bypasses active *module* activation (`module_activation`).

## Current behavior (PR1)

| Operation | Active plugin | Active module |
|---|---|---|
| `numan remove` | Always refused (Issue #22 hint) | Refused unless `--force` |
| `numan update` | Refused (Issue #22 hint) | Refused (use `deactivate` first) |
| `numan deactivate` | Deferred (plugin deactivate lands in a later PR) | Supported today |

`numan doctor` reports an **info** finding `activation.plugin_mutation_gated` for each package with `package_type == "plugin"` and `activation.is_some()` (even when `nu_state/paths.json` is missing). Remediation in this slice: keep the package installed or install without activating; plugin deactivate is deferred to PR2+.

Canonical hint text lives in `util::hints::active_plugin_mutation_gated` / `ACTIVE_PLUGIN_MUTATION_GATED_FIX`.
## Deferred (PR2+)

- Plugin deactivation that clears the activation record safely
- Re-enable Stage 1 acceptance remove/gc after deactivate exists
- Full safety matrix from [Issue #22](https://github.com/tonythethompson/numan/issues/22)

See also: [docs/acceptance/official-registry-stage1.md](acceptance/official-registry-stage1.md).
