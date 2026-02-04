#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -eu

# RUNTIME_PATH is substituted by the Nix build.
export PATH=__RUNTIME_PATH__

XFSTESTS_DIR=/opt/xfstests
cd "$XFSTESTS_DIR"

TEST_DEV=${XFSTESTS_TEST_DEV:-/dev/vdc}
SCRATCH_DEV=${XFSTESTS_SCRATCH_DEV:-/dev/vdd}
export TEST_DEV SCRATCH_DEV

# Mount xfstests images with explicit error checking so a mount failure is not
# silently skipped (which would cause ./check to run against empty directories
# and still print the "all passed" success line).
for entry in "$TEST_DEV:$XFSTESTS_DIR/test:test" "$SCRATCH_DEV:$XFSTESTS_DIR/scratch:scratch"; do
    dev="${entry%%:*}"; rest="${entry#*:}"; mnt="${rest%%:*}"; role="${rest##*:}"
    if [ ! -b "$dev" ]; then
        echo "Expected $dev to be a block device for xfstests $role" >&2
        exit 1
    fi
    if ! mount -t ext2 "$dev" "$mnt"; then
        echo "Failed to mount $dev on $mnt ($role)" >&2
        exit 1
    fi
    if ! mountpoint -q "$mnt"; then
        echo "$mnt is not a mountpoint after mount(8) succeeded ($role)" >&2
        exit 1
    fi
done

RUNLIST_FILE=""
TEST_ARGS=""

# Parse -R flag and collect direct test names.
# Test names are simple identifiers (e.g. "generic/001") so accumulating
# them in a space-separated string is safe.
while [ $# -gt 0 ]; do
  case "$1" in
    -R|--runlist)
      if [ $# -lt 2 ]; then
        echo "Error: -R|--runlist requires a filename argument." >&2
        exit 2
      fi
      RUNLIST_FILE="$2"
      shift 2
      ;;
    --)
      shift
      TEST_ARGS="$TEST_ARGS $*"
      break
      ;;
    *)
      TEST_ARGS="$TEST_ARGS $1"
      shift
      ;;
  esac
done

if [ -n "$RUNLIST_FILE" ]; then
  if [ ! -f "$RUNLIST_FILE" ]; then
    echo "Run list file not found: $RUNLIST_FILE" >&2
    exit 2
  fi
  while IFS= read -r test; do
    case "$test" in
      ""|\#*) continue ;;
    esac
    TEST_ARGS="$TEST_ARGS $test"
  done < "$RUNLIST_FILE"
fi

# Prepend block-list exclusion so blocked tests are skipped.
if [ -f "$XFSTESTS_DIR/block.list" ]; then
    TEST_ARGS="-E $XFSTESTS_DIR/block.list $TEST_ARGS"
fi

# Word-splitting is intentional here: TEST_ARGS contains only test names
# and the -E flag, none of which contain whitespace or shell metacharacters.
# shellcheck disable=SC2086
./check $TEST_ARGS
