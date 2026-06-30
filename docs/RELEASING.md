# Releasing Numan

## Versioning

- Follow [Semantic Versioning](https://semver.org/).
- Single source of truth: `version` in `Cargo.toml` (crate `numan-cli`, binary `numan`).
- Git tags use a `v` prefix: `v0.1.0`.

## Changelog

- Maintain [CHANGELOG.md](../CHANGELOG.md) using [Keep a Changelog](https://keepachangelog.com/).
- Move items from `[Unreleased]` into a dated version section before tagging.
- GitHub Release notes are auto-generated; the changelog is the human-curated record.

## Release checklist

1. Bump `version` in `Cargo.toml`.
2. Update `CHANGELOG.md` (new section, clear `[Unreleased]`).
3. Merge to `master` and verify CI is green.
4. Tag and push:

   ```bash
   git tag v0.1.1
   git push origin v0.1.1
   ```

5. Confirm the [Release workflow](https://github.com/tonythethompson/numan/actions/workflows/release.yml) uploads platform archives and `SHA256SUMS`.
6. Confirm the **Publish to crates.io** job succeeds (requires `CRATES_IO_TOKEN` repository secret).

## crates.io

- Package name: **`numan-cli`** (install with `cargo install numan-cli`).
- Binary name: **`numan`**.
- Set `CRATES_IO_TOKEN` in GitHub repository secrets before the first publish.
- Dry-run locally: `cargo publish --dry-run`.

## Install paths users should see

| Method | Command |
|--------|---------|
| GitHub Release | Download archive from [Releases](https://github.com/tonythethompson/numan/releases) |
| crates.io | `cargo install numan-cli` |
| From source | `cargo install --path .` or `cargo install --git https://github.com/tonythethompson/numan` |
