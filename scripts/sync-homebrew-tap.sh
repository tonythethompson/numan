#!/usr/bin/env bash
# Sync packaging/homebrew/numan.rb to the homebrew-numan tap repository.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FORMULA_SRC="${ROOT}/packaging/homebrew/numan.rb"
TAP_REPO="${TAP_REPO:-${HOME}/src/homebrew-numan}"

if [[ ! -d "${TAP_REPO}/.git" ]]; then
  echo "Clone the tap first: git clone git@github.com:tonythethompson/homebrew-numan.git ${TAP_REPO}"
  exit 1
fi

mkdir -p "${TAP_REPO}/Formula"
cp "${FORMULA_SRC}" "${TAP_REPO}/Formula/numan.rb"
echo "Copied formula to ${TAP_REPO}/Formula/numan.rb"
echo "Commit and push in the tap repo to publish."
