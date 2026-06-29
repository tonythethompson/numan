# nupm compatibility audit (Phase 6.0)

This document is the **Phase 6.0 compatibility audit** for Numan's read-only nupm discovery and one-way module import work. It pins upstream nupm behavior, defines the narrow profile Numan will support, and inventories the checked-in fixture corpus under `tests/fixtures/nupm/`.

**No import or CLI implementation ships in Phase 6.0.** Phase 6.1+ must treat this profile as the contract.

Related planning: [`Phase6Plan.md`](../Phase6Plan.md) §4–§5, §13.

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

nupm locates a package root by walking parents from a path until `nupm.nuon` is found (`find-root` in `dirs.nu`). Numan inspect will accept either an explicit package directory or a path inside the tree.

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

Numan's constrained parser (Phase 6.1) supports the subset listed in `Phase6Plan.md` §7.1a. Anything else (closures, `$foo`, dates, binary literals) is **InvalidMetadata**.

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

`description`, `license`, `version` — parsed and displayed in `inspect`; only `name` must match directory layout.

---

## Rejected format profile

| Condition | Compatibility class | User-visible reason (inspect) |
|-----------|---------------------|-------------------------------|
| Unparseable NUON / closure / `$var` | `InvalidMetadata` | Metadata uses unsupported NUON constructs |
| Missing `name`, `version`, or `type` | `InvalidMetadata` | Required metadata keys missing |
| `type: script` | `DeferredScript` | Script packages are not imported in Phase 6 |
| `type: custom` | `UnsupportedCustomBuild` | Custom install / build.nu packages are not supported |
| Unknown `type` | `UnknownType` | Unknown package type |
| `deps` / external dependency record | `UnsupportedDependencies` | External dependency metadata is not supported |
| `scripts: [...]` on module | `DeferredScript` | Auxiliary scripts are not imported with modules |
| `build.nu` present | `UnsupportedCustomBuild` | build.nu must not be present |
| Missing `<name>/` directory | `UnsafeFilesystemLayout` | Module directory missing |
| Missing `mod.nu` | `UnsafeFilesystemLayout` | Module entry mod.nu missing |
| Symlink / reparse-point escape | `UnsafeFilesystemLayout` | Unsafe filesystem layout |
| Path outside package root | `UnsafeFilesystemLayout` | Entry path outside package root |

Rejected packages remain **read-only visible** in `status` / `inspect`; Numan never mutates nupm trees.

---

## Fixture package inventory

Corpus root: `tests/fixtures/nupm/`.

### Supported (`supported/`)

| Fixture | Layout | Expected Phase 6.1 class |
|---------|--------|------------------------|
| `minimal-module/` | Standard module; bare identifiers; no optional fields | `ImportableModule` |
| `module-with-metadata/` | Adds `description`, `license` | `ImportableModule` |

### Rejected (`rejected/`)

| Fixture | Based on | Expected class |
|---------|----------|----------------|
| `script-type/` | nupm `spam_script` | `DeferredScript` |
| `custom-with-build/` | nupm `spam_custom` | `UnsupportedCustomBuild` |
| `custom-without-build/` | custom type, no build.nu | `UnsupportedCustomBuild` |
| `missing-mod-nu/` | module dir without mod.nu | `UnsafeFilesystemLayout` |
| `module-with-scripts/` | nupm `spam_module` | `DeferredScript` |
| `unknown-type/` | fictional `overlay` type | `UnknownType` |
| `malformed-closure/` | closure in metadata | `InvalidMetadata` |
| `external-deps/` | `deps` record | `UnsupportedDependencies` |
| `missing-required-keys/` | no `name` key | `InvalidMetadata` |

### Layout sample (`nupm-home-layout/`)

Simulates `$NUPM_HOME` after installing one module and one script:

```text
nupm-home-layout/
  modules/minimal-module/     ← import candidate
  scripts/example-script.nu   ← not scanned for import
```

Use as `--nupm-home` target in Phase 6.1 integration tests.

---

## NUPM_HOME discovery (Numan commands)

Numan **must not** guess nupm's default config path.

Resolution order for `numan nupm status|inspect|import`:

1. `--nupm-home PATH`
2. `NUPM_HOME` environment variable
3. Error with guidance (no silent fallback)

---

## Platform path behavior

| Topic | Windows | Linux | macOS |
|-------|---------|-------|-------|
| Path separators | `\` in UI; normalize for comparisons | `/` | `/` |
| `NUPM_HOME` | User-set; often under `%LOCALAPPDATA%` or `%APPDATA%` | XDG or `~/.config/nupm` | Same as Linux |
| Symlinks | Reject reparse-point / junction escapes on copy | Reject symlink escape | Reject symlink escape |
| Unicode paths | Must round-trip in inspect output | Same | Same |
| Case sensitivity | Case-insensitive FS common; compare paths canonically | Case-sensitive | Case-sensitive default |

Phase 6.0 fixtures use ASCII paths; Phase 6.4 adds Unicode / space acceptance tests.

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
| T05 | Parser | random bytes (fuzz) | No panic; Err |
| T06 | Classify | `supported/minimal-module/` | ImportableModule |
| T07 | Classify | `rejected/script-type/` | DeferredScript |
| T08 | Classify | `rejected/custom-with-build/` | UnsupportedCustomBuild |
| T09 | Classify | `rejected/module-with-scripts/` | DeferredScript |
| T10 | Classify | `rejected/external-deps/` | UnsupportedDependencies |
| T11 | Classify | `rejected/missing-mod-nu/` | UnsafeFilesystemLayout |
| T12 | Classify | `rejected/unknown-type/` | UnknownType |
| T13 | Discovery | `nupm-home-layout/` + `--nupm-home` | 1 supported module, 0 imports |
| T14 | Discovery | no `--nupm-home`, no env | Clear error message |
| T15 | Safety | inspect/status on fixtures | No writes under fixture dirs |
| T16 | Import | supported module (Phase 6.2) | Payload under `$NUMAN_ROOT` only |
| T17 | Import | any rejected fixture (Phase 6.2) | Error; nupm bytes unchanged |
| T18 | Drift | re-import / diff (Phase 6.3) | Per Phase6Plan §10 |
| T19 | Platform | Unicode path (Phase 6.4) | Inspect + import |
| T20 | Platform | symlink escape attempt (Phase 6.4) | Rejected at copy |

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
| 2026-06-28 | Initial Phase 6.0 audit; pin `421eee1c`; fixture corpus + this document |
