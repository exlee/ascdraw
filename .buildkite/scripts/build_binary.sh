#!/usr/bin/env bash
# Builds the release binary and packs it into dist/$ARTIFACT_NAME.tar.gz.
# Checks the glibc requirement when MAX_GLIBC is set.
set -euo pipefail

: "${ARTIFACT_NAME:?ARTIFACT_NAME must be set}"

if [[ "$(uname -s)" == "Linux" ]]; then
  packages=(
    libfontconfig1-dev
    libwayland-dev
    libx11-dev
    libx11-xcb-dev
    libxcb-render0-dev
    libxcb-shape0-dev
    libxcb-xfixes0-dev
    libxkbcommon-dev
  )
  missing=()
  for package in "${packages[@]}"; do
    dpkg -s "$package" >/dev/null 2>&1 || missing+=("$package")
  done
  if (( ${#missing[@]} > 0 )); then
    echo "--- :linux: Installing build dependencies: ${missing[*]}"
    sudo=()
    (( EUID == 0 )) || sudo=(sudo)
    "${sudo[@]}" apt-get update
    "${sudo[@]}" apt-get install -y "${missing[@]}"
  fi
fi

source .buildkite/scripts/setup_rust.sh

echo "--- :cargo: Building release binary"
cargo build --release --locked

if [[ -n "${MAX_GLIBC:-}" ]]; then
  echo "--- :linux: Checking glibc requirement"
  ruby scripts/check_glibc_version.rb target/release/ascdraw "$MAX_GLIBC"
fi

echo "--- :package: Staging ${ARTIFACT_NAME}.tar.gz"
mkdir -p dist
tar -czf "dist/${ARTIFACT_NAME}.tar.gz" -C target/release ascdraw
