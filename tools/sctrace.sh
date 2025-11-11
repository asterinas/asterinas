#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e

# Get the script directory
SCRIPT_PATH="$(readlink -f "${BASH_SOURCE[0]}")"
SCRIPT_DIR="$(dirname "$SCRIPT_PATH")"
ASTERINAS_ROOT="$(dirname "$SCRIPT_DIR")"

# Search all SCML files
scml_files=$(find "$ASTERINAS_ROOT/book/src/kernel/linux-compatibility" -type f -name "*.scml" | tr '\n' ' ')

# Run sctrace with all arguments passed to this script
cargo run -q --manifest-path "$ASTERINAS_ROOT/tools/sctrace/Cargo.toml" -- $scml_files "$@"
