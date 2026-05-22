#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -eu

# RUNTIME_PATH is substituted by the Nix build.
export PATH=__RUNTIME_PATH__

XFSTESTS_DIR=/opt/xfstests
BACKEND=${XFSTESTS_BACKEND:-block}
export XFSTESTS_DIR XFSTESTS_BACKEND

case "$BACKEND" in
    block|virtiofs)
        BACKEND_RUNNER="$XFSTESTS_DIR/run_xfstests_${BACKEND}.sh"
        ;;
    *)
        echo "Unsupported xfstests backend: $BACKEND" >&2
        exit 1
        ;;
esac

if [ ! -x "$BACKEND_RUNNER" ]; then
    echo "xfstests backend runner is not executable: $BACKEND_RUNNER" >&2
    exit 1
fi

mkdir -p "$XFSTESTS_DIR/test" "$XFSTESTS_DIR/scratch"
cd "$XFSTESTS_DIR"

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
"$BACKEND_RUNNER" $TEST_ARGS
