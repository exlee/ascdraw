#!/usr/bin/env bash
# Installs the given apt packages that are not installed yet.
# Uses sudo only when the job does not run as root.
set -euo pipefail

missing=()
for package in "$@"; do
  dpkg -s "$package" >/dev/null 2>&1 || missing+=("$package")
done

if (( ${#missing[@]} == 0 )); then
  exit 0
fi

sudo=()
(( EUID == 0 )) || sudo=(sudo)

echo "--- :ubuntu: Installing ${missing[*]}"
"${sudo[@]}" apt-get update
"${sudo[@]}" env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "${missing[@]}"
