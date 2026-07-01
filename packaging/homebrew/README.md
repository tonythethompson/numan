# Homebrew

## Install (no tap required)

```bash
brew install --formula https://raw.githubusercontent.com/tonythethompson/numan/master/packaging/homebrew/numan.rb
```

Requires a [Nushell](https://www.nushell.sh/) binary on `PATH` for `numan init` and `numan activate`.

## Optional tap workflow

To publish a dedicated tap (e.g. `tonythethompson/homebrew-numan`):

1. Create a repository named `homebrew-numan`.
2. Copy `numan.rb` to `Formula/numan.rb` in that repo.
3. Users install with:

   ```bash
   brew tap tonythethompson/numan
   brew install numan
   ```

## Updating for a release

Bump `version` and per-platform `sha256` in `numan.rb` from the GitHub Release `SHA256SUMS` file. See [docs/PACKAGING.md](../../docs/PACKAGING.md).
