#!/usr/bin/env bash
# Builds Ascdraw.app and packs it into dist/Ascdraw.dmg.
set -euo pipefail

CARGO_BUNDLE_VERSION="0.11.0"

source .buildkite/scripts/setup_rust.sh

echo "--- :homebrew: Installing packaging tools"
for formula in imagemagick create-dmg; do
  brew list --formula "$formula" >/dev/null 2>&1 || brew install "$formula"
done

if ! cargo install --list | grep -q "^cargo-bundle v${CARGO_BUNDLE_VERSION}:"; then
  echo "--- :cargo: Installing cargo-bundle ${CARGO_BUNDLE_VERSION}"
  cargo install cargo-bundle --version "$CARGO_BUNDLE_VERSION" --locked
fi

echo "--- :art: Generating macOS icon"
rake icon

echo "--- :mac: Building macOS app bundle"
rake bundle

echo "--- :package: Creating DMG"
mkdir -p dist
rm -f dist/Ascdraw.dmg
create-dmg \
  --volname "Ascdraw" \
  --window-pos 200 120 \
  --window-size 640 420 \
  --icon-size 96 \
  --icon "Ascdraw.app" 180 170 \
  --app-drop-link 460 170 \
  "dist/Ascdraw.dmg" \
  "target/release/bundle/osx/Ascdraw.app"
