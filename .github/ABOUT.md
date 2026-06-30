# GitHub repository About metadata

Use this when configuring the repository **About** panel on GitHub (Settings → General → or the gear icon on the repo home page).

## Description

Cross-platform package manager for Nushell — verified registry artifacts, lockfiles, safe plugin/module activation, and nupm import.

## Website

Leave blank until a project site or docs URL is published.

## Topics

```
nushell
nushell-plugin
package-manager
rust
cli
lockfile
nupm
cross-platform
```

## Suggested social preview

The README opening summarizes the project for link previews. Optionally add a `social-preview.png` under `.github/` (1280×640) when branding assets exist.

## Apply via GitHub CLI

```bash
gh repo edit \
  --description "Cross-platform package manager for Nushell — verified registry artifacts, lockfiles, safe plugin/module activation, and nupm import." \
  --add-topic nushell \
  --add-topic nushell-plugin \
  --add-topic package-manager \
  --add-topic rust \
  --add-topic cli \
  --add-topic lockfile \
  --add-topic nupm \
  --add-topic cross-platform
```
