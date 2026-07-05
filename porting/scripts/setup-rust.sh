#!/bin/bash
set -e
export RUSTUP_HOME=/home/qute-wsl/.rustup
export CARGO_HOME=/home/qute-wsl/.cargo
export PATH="$CARGO_HOME/bin:$PATH"

# Install rustup if not present
if [ ! -f "$CARGO_HOME/bin/rustup" ]; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none
fi

source "$CARGO_HOME/env"
rustup install nightly-2026-04-03
rustup default nightly-2026-04-03

# Install cargo-osdk
cargo install cargo-osdk

echo "Rust setup complete"
