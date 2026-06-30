# Phase 7 Plan: Polish, CI, Distribution

Take Numan from feature-complete core (Phases 1–6) to a distributable, polished CLI.

**Tracking issue:** [#12](https://github.com/tonythethompson/numan/issues/12)

---

## Status overview

| Slice | Theme | Status |
|-------|--------|--------|
| 7.1 | Distribution baseline | ✅ Done |
| 7.2 | `numan doctor` | 📋 Specified — [docs/numan-doctor.md](docs/numan-doctor.md) |
| 7.3 | Daily-driver polish | 🚧 Partial — completions + error UX shipped; `--help` audit remains |
| 7.4 | Onboarding path | 🔜 Planned |
| 7.5 | CI / release hardening | 🔜 Planned |
| 7.6 | Wider distribution | 🔜 Optional |

---

## 7.1 Distribution baseline ✅

Shipped in v0.1.0–v0.1.1:

- GitHub Release workflow (linux / windows / macOS × aarch64 + x86_64 mac)
- `CHANGELOG.md` + [docs/RELEASING.md](docs/RELEASING.md)
- crates.io publish (`cargo install numan-cli`)
- `numan init` / `numan init --refresh`
- Real-Nu acceptance CI job
- README install + quickstart (partial)

---

## 7.2 `numan doctor` 📋

**Spec:** [docs/numan-doctor.md](docs/numan-doctor.md)

Read-only health command aggregating:

- Nu path drift (`NuPaths`)
- Pending journals (plugin, autoload, lifecycle)
- Lockfile activation identity + autoload projection
- Payload directory presence
- Registry configuration
- Optional nupm coexistence (`scan_on_doctor` config flag)

**`numan doctor --fix`** applies safe repairs (layout, `init`, `registry sync`, `init --refresh`, `activate` recovery) via existing commands — see repair tiers in spec.

**Why now:** Scattered checks (`activate --check`, journal recovery, `nupm status`) need a single entry point before error-UX and onboarding polish.

**Deliverables:**

- `src/cmd/doctor.rs`, wire in `main.rs`
- `tests/doctor_test.rs`
- README + `AGENTS.md` updates

**Non-goals:** install/remove/update/gc, nupm import, lifecycle journal auto-completion, foreign managed-file overwrite.

---

## 7.3 Daily-driver polish 🚧

Shipped (partial):

1. **Shell completions** — `numan completions bash|fish|zsh|powershell` via `clap_complete`
2. **Error message UX pass** — `init`, `install`, `activate`, `nupm import` use `util::hints` aligned with doctor `fix` strings

Remaining:

3. **`--help` audit** — README command table vs clap flags

---

## 7.4 Onboarding path 🔜

1. **`numan init` polish** — optional guided registry add after first init
2. **README** — single copy-paste path: install → init → registry → search → install
3. **Compatibility matrix** — link/promote `docs/nupm-compatibility.md` from README

---

## 7.5 CI / release hardening 🔜

1. Gate release workflow on green `ci.yml`
2. MSRV pin in `Cargo.toml` + CI
3. Optional: `cargo deny`, `cargo package` on PRs
4. Release checklist: `cargo fmt --check` before tag (lesson from v0.1.1)

---

## 7.6 Wider distribution (optional) 🔜

- Homebrew tap, Winget, Scoop manifests
- Document `cargo install --git`

Lower priority until install-channel demand exists.

---

## Deferred (not Phase 7)

**Phase 5** ([#11](https://github.com/tonythethompson/numan/issues/11)): source builds (5.2), snapshots/rollback (5.3), plugin lifecycle gate (5.5).

---

## Recommended implementation order

```text
7.1 ✅ → 7.2 doctor → 7.3 polish → 7.4 onboarding → 7.5 CI hardening → 7.6 packagers
```
