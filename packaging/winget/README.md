# Windows Package Manager (winget)

Manifests follow the [winget-pkgs](https://github.com/microsoft/winget-pkgs) layout under `manifests/t/TonyTheThompson/Numan/<version>/`.

## Install from local manifests (before winget-pkgs merge)

```powershell
winget install --manifest .\packaging\winget\manifests\t\TonyTheThompson\Numan\0.1.2
```

Run from the repository root, or pass the full path to the version directory.

## Install from winget community repository

After manifests are accepted in [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs):

```powershell
winget install TonyTheThompson.Numan
```

## Submitting an update

1. Copy `manifests/t/TonyTheThompson/Numan/<version>/` into a fork of `microsoft/winget-pkgs` at the same path.
2. Open a PR using [wingetcreate](https://github.com/microsoft/winget-cli/blob/master/doc/windows/package-manager/winget/create.md) or manually.
3. Update `InstallerSha256` from the release `SHA256SUMS` file (uppercase hex).

See [docs/PACKAGING.md](../../docs/PACKAGING.md) for the full release checklist.
