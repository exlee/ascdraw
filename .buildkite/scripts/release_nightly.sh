#!/usr/bin/env bash
# Publishes the nightly artifacts as a GitHub prerelease. gh resolves the
# repository from the checkout's origin remote.
set -euo pipefail

GH_TOKEN="$(buildkite-agent secret get GH_TOKEN)"
export GH_TOKEN

echo "--- :buildkite: Downloading artifacts"
buildkite-agent artifact download "dist/*" .

tag="nightly-${BUILDKITE_BUILD_NUMBER}"

# Tag the built commit, not whatever the branch moved to meanwhile;
# the next run compares against this tag.
echo "--- :github: Creating release ${tag} at ${BUILDKITE_COMMIT}"
gh release create "$tag" \
  --target "$BUILDKITE_COMMIT" \
  --title "Nightly ${BUILDKITE_BUILD_NUMBER}" \
  --prerelease \
  --generate-notes \
  dist/ascdraw-linux.tar.gz \
  dist/ascdraw-macos.tar.gz \
  dist/Ascdraw.dmg
