#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

# Usage: ./add_test.sh <test_name>
# This script creates a new test case by generating a subdirectory
# containing the necessary Nix configuration file and a symlink to
# a generated test script file.

set -e

if [ "$#" -ne 1 ]; then
    echo "Usage: $0 <test_name>"
    exit 1
fi

TEST_NAME="$1"
TEST_DIR="${TEST_NAME}"
OVERLAY_DIR="../etc_nixos/overlays/test-asterinas"

# Create the main directory for the test configuration
echo "Creating test configuration directory: ${TEST_DIR}/"
mkdir -p "$TEST_DIR"

# Define file paths
NIX_CONFIG_FILE="${TEST_DIR}/configuration.nix"
ACTUAL_SCRIPT_FILE="${OVERLAY_DIR}/test-${TEST_NAME}.sh"
SYMLINK_PATH="${TEST_DIR}/test-${TEST_NAME}.sh"
SYMLINK_TARGET="../../etc_nixos/overlays/test-asterinas/test-${TEST_NAME}.sh"

# --- Create Nix Configuration File ---
echo "Creating Nix config: ${NIX_CONFIG_FILE}"
cat > "$NIX_CONFIG_FILE" <<EOF
{ config, lib, pkgs, ... }:

{
  # Add test-specific NixOS configuration here.
  # For example:
  # services.xserver.enable = true;
}
EOF

# --- Create Actual Test Script File ---
echo "Creating actual test script: ${ACTUAL_SCRIPT_FILE}"
cat > "$ACTUAL_SCRIPT_FILE" <<EOF
#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

. test-framework.sh

start_test "${TEST_NAME}"

# Add your test steps here.

finish_test
EOF

chmod +x "$ACTUAL_SCRIPT_FILE"

# --- Create Symlink to the Test Script ---
echo "Creating symlink: ${SYMLINK_PATH} -> ${SYMLINK_TARGET}"
ln -sf "$SYMLINK_TARGET" "$SYMLINK_PATH"