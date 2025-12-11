#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e

# Get the script directory
SCRIPT_PATH="$(readlink -f "${BASH_SOURCE[0]}")"
SCRIPT_DIR="$(dirname "$SCRIPT_PATH")"
ASTERINAS_ROOT="$(dirname "$SCRIPT_DIR")"

# Run sctrace with all arguments passed to this script
cargo run -q --manifest-path "$ASTERINAS_ROOT/tools/sctrace/Cargo.toml" -- "$@"
