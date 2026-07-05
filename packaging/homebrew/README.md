# Homebrew

## Install via tap (recommended)

```bash
brew tap tonythethompson/numan
brew install numan
```

Tap repository: [github.com/tonythethompson/homebrew-numan](https://github.com/tonythethompson/homebrew-numan)

## Install without a tap

```bash
brew install --formula https://raw.githubusercontent.com/tonythethompson/numan/master/packaging/homebrew/numan.rb
```

Requires a [Nushell](https://www.nushell.sh/) binary on `PATH` for `numan init` and `numan activate`.

## Updating for a release

Bump `version` and per-platform `sha256` in `numan.rb` from the GitHub Release `SHA256SUMS` file. Sync the same file to `homebrew-numan/Formula/numan.rb` (see `scripts/sync-homebrew-tap.sh`).
