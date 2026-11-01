#!/usr/bin/env bash
# Uploads .buildkite/nightly.yml when a nightly build is due.
set -euo pipefail

.buildkite/scripts/apt_install.sh ruby
.buildkite/scripts/setup_gh.sh

GH_TOKEN="$(buildkite-agent secret get GH_TOKEN)"
export GH_TOKEN

ruby scripts/nightly_changes.rb "$BUILDKITE_SOURCE" "$BUILDKITE_COMMIT" .buildkite/nightly.yml
