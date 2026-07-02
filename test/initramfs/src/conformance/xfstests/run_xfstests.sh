#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -eu

# RUNTIME_PATH is substituted by the Nix build.
export PATH=__RUNTIME_PATH__

XFSTESTS_FS_TYPE=${XFSTESTS_FS_TYPE:-ext2}
export FSTYP="$XFSTESTS_FS_TYPE"

XFSTESTS_DIR=/opt/xfstests
XFSTESTS_FS_DIR="$XFSTESTS_DIR/$XFSTESTS_FS_TYPE"
XFSTESTS_CONFIG="$XFSTESTS_FS_DIR/config/xfstests.config"
XFSTESTS_PREPARE="$XFSTESTS_FS_DIR/prepare.sh"
XFSTESTS_BLOCK_LIST="$XFSTESTS_FS_DIR/run_list/block.list"
cd "$XFSTESTS_DIR"

if [ ! -d "$XFSTESTS_FS_DIR" ]; then
    echo "Unsupported xfstests filesystem type: $XFSTESTS_FS_TYPE" >&2
    exit 2
fi
if [ ! -f "$XFSTESTS_CONFIG" ]; then
    echo "Missing xfstests config: $XFSTESTS_CONFIG" >&2
    exit 2
fi
if [ ! -f "$XFSTESTS_PREPARE" ]; then
    echo "Missing xfstests preparation script: $XFSTESTS_PREPARE" >&2
    exit 2
fi

export HOST_OPTIONS="$XFSTESTS_CONFIG"

# Prepare scripts consume the same filesystem variables that `./check` later
# loads through `HOST_OPTIONS`.
# shellcheck source=/dev/null
. "$XFSTESTS_CONFIG"

# shellcheck source=/dev/null
. "$XFSTESTS_PREPARE"

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
    test=${test%%#*}
    case "$test" in
      *[![:space:]]*) ;;
      *) continue ;;
    esac
    TEST_ARGS="$TEST_ARGS $test"
  done < "$RUNLIST_FILE"
fi

# Prepend block-list exclusion so blocked tests are skipped.
if [ -f "$XFSTESTS_BLOCK_LIST" ]; then
    TEST_ARGS="-E $XFSTESTS_BLOCK_LIST $TEST_ARGS"
fi

# Word-splitting is intentional here: TEST_ARGS contains only test names
# and the -E flag, none of which contain whitespace or shell metacharacters.
# shellcheck disable=SC2086
./check $TEST_ARGS
