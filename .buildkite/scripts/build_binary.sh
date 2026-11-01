#!/usr/bin/env bash
# Builds the release binary and packs it into dist/$ARTIFACT_NAME.tar.gz.
# Checks the glibc requirement when MAX_GLIBC is set.
set -euo pipefail

: "${ARTIFACT_NAME:?ARTIFACT_NAME must be set}"

if [[ "$(uname -s)" == "Linux" ]]; then
  .buildkite/scripts/apt_install.sh \
    curl \
    ca-certificates \
    build-essential \
    pkg-config \
    ruby \
    libfontconfig1-dev \
    libwayland-dev \
    libx11-dev \
    libx11-xcb-dev \
    libxcb-render0-dev \
    libxcb-shape0-dev \
    libxcb-xfixes0-dev \
    libxkbcommon-dev
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
