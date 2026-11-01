#!/usr/bin/env bash
# Installs the GitHub CLI from the official apt repository when the agent
# has none. The Ubuntu archive version is too old for `gh release create
# --generate-notes`.
set -euo pipefail

if command -v gh >/dev/null 2>&1; then
  exit 0
fi

.buildkite/scripts/apt_install.sh ca-certificates curl

sudo=()
(( EUID == 0 )) || sudo=(sudo)
keyring=/etc/apt/keyrings/githubcli-archive-keyring.gpg

echo "--- :github: Installing GitHub CLI"
"${sudo[@]}" mkdir -p -m 755 /etc/apt/keyrings
curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
  | "${sudo[@]}" tee "$keyring" >/dev/null
"${sudo[@]}" chmod go+r "$keyring"
echo "deb [arch=$(dpkg --print-architecture) signed-by=${keyring}] https://cli.github.com/packages stable main" \
  | "${sudo[@]}" tee /etc/apt/sources.list.d/github-cli.list >/dev/null
"${sudo[@]}" apt-get update
"${sudo[@]}" env DEBIAN_FRONTEND=noninteractive apt-get install -y gh
gh --version
