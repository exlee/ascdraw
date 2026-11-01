# shellcheck shell=bash
# Sourced by build scripts. Puts the stable Rust toolchain on PATH,
# installing rustup first when the agent has none.

if [[ -f "$HOME/.cargo/env" ]]; then
  # shellcheck source=/dev/null
  source "$HOME/.cargo/env"
fi

if ! command -v rustup >/dev/null 2>&1; then
  echo "--- :rust: Installing rustup"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain none
  # shellcheck source=/dev/null
  source "$HOME/.cargo/env"
fi

echo "--- :rust: Updating stable toolchain"
rustup toolchain install stable --profile minimal
rustup default stable
rustc --version
