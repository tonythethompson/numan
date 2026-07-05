# Packaging (Homebrew, winget)

Third-party install manifests live under `packaging/`. They pin GitHub Release binaries and must be updated on each version tag.

## Release packaging checklist

After a GitHub Release is published (see [RELEASING.md](RELEASING.md)):

1. Download `SHA256SUMS` from the release assets.
2. **Homebrew** — edit `packaging/homebrew/numan.rb`:
   - Bump `version`
   - Update each platform `sha256` (lowercase hex)
3. **winget** — add `packaging/winget/manifests/t/tonythethompson/Numan/<version>/` with three manifests (schema **1.12.0**):
   - `tonythethompson.Numan.yaml` (version)
   - `tonythethompson.Numan.installer.yaml`
   - `tonythethompson.Numan.locale.en-US.yaml`
   - Set `PackageIdentifier` to `tonythethompson.Numan` (lowercase publisher segment)
   - Set `InstallerSha256` to uppercase hex from `SHA256SUMS`
   - Update nested `RelativeFilePath` if the archive folder name changed
4. Open a PR to [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs) for community winget installs (use WSL/Linux so `manifests/t/tonythethompson/` path casing is preserved).
5. **Homebrew tap** — sync `packaging/homebrew/numan.rb` to [tonythethompson/homebrew-numan](https://github.com/tonythethompson/homebrew-numan) `Formula/numan.rb` (`scripts/sync-homebrew-tap.sh`).

## Install channels

| Channel | Command |
|---------|---------|
| GitHub Release | Download archive from [Releases](https://github.com/tonythethompson/numan/releases) |
| crates.io | `cargo install numan-cli` |
| From git | `cargo install --git https://github.com/tonythethompson/numan` |
| Homebrew (tap) | `brew tap tonythethompson/numan && brew install numan` |
| Homebrew (direct) | `brew install --formula https://raw.githubusercontent.com/tonythethompson/numan/master/packaging/homebrew/numan.rb` |
| winget (local manifest) | `winget install --manifest packaging/winget/manifests/t/tonythethompson/Numan/<version>` |
| winget (community) | `winget install tonythethompson.Numan` (after winget-pkgs PR merges) |

## Archive layout

Release archives extract to `numan-<version>-<target>/` containing the `numan` (or `numan.exe`) binary. Formulas and winget nested installers assume this layout.

## Platform coverage

| Platform | Release asset | Homebrew | winget |
|----------|---------------|----------|--------|
| Linux x86_64 | `.tar.gz` | yes | — |
| macOS Apple Silicon | `.tar.gz` | yes | — |
| macOS Intel | `.tar.gz` | yes | — |
| Windows x86_64 | `.zip` | — | yes |

Scoop is not packaged yet; see [Phase7Plan.md](../Phase7Plan.md).
