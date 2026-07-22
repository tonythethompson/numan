# Windows Package Manager (winget)

Manifests follow the [winget-pkgs](https://github.com/microsoft/winget-pkgs) layout under `manifests/t/tonythethompson/numan/<version>/`.

Package path and identifier use lowercase `tonythethompson.numan` (same publisher folder as [tonythethompson.QuickShell](https://github.com/microsoft/winget-pkgs/tree/master/manifests/t/tonythethompson/QuickShell); package segment is lowercase to avoid Windows casing duplicates).

## Install from local manifests (before winget-pkgs merge)

```powershell
winget install --manifest .\packaging\winget\manifests\t\tonythethompson\numan\0.1.4
```

Run from the repository root, or pass the full path to the version directory.

## Install from winget community repository

After manifests are accepted in [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs):

```powershell
winget install tonythethompson.numan
```

## Submitting an update

1. Copy `manifests/t/tonythethompson/numan/<version>/` into a fork of `microsoft/winget-pkgs` at the same path (use WSL/Linux to preserve path casing).
2. Open a PR using [wingetcreate](https://github.com/microsoft/winget-cli/blob/master/doc/windows/package-manager/winget/create.md) or manually.
3. Use manifest schema **1.12.0** (`$schema` URLs and `ManifestVersion` must match).
4. Update `InstallerSha256` from the release `SHA256SUMS` file (uppercase hex).
5. **One version per PR** to winget-pkgs — include only the new version directory; do not mix `0.1.3` and `0.1.4` in one PR.

See [docs/PACKAGING.md](../../docs/PACKAGING.md) for the full release checklist.
