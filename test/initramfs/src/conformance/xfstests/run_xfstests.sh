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
REQUESTED_TESTS=$(mktemp)
SELECTED_TESTS=$(mktemp)

trap 'rm -f "$REQUESTED_TESTS" "$SELECTED_TESTS"' EXIT

# Parse -R flag and collect direct test names.
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
      while [ $# -gt 0 ]; do
        printf '%s\n' "$1" >> "$REQUESTED_TESTS"
        shift
      done
      break
      ;;
    *)
      printf '%s\n' "$1" >> "$REQUESTED_TESTS"
      shift
      ;;
  esac
done

# When a selector is set, run only the selected test ids and ignore the blocklist.
CONFORMANCE_TEST_SELECTOR=${CONFORMANCE_TEST_SELECTOR:-}

if [ -n "$CONFORMANCE_TEST_SELECTOR" ]; then
  printf '%s\n' "$CONFORMANCE_TEST_SELECTOR" | tr ',' '\n' > "$REQUESTED_TESTS"

  selected_count=0
  invalid_selector=0
  while IFS= read -r test_name || [ -n "$test_name" ]; do
    test_name=${test_name#"${test_name%%[![:space:]]*}"}
    test_name=${test_name%"${test_name##*[![:space:]]}"}
    [ -z "$test_name" ] && continue

    selected_count=$((selected_count + 1))
    case "$test_name" in
      /*|-*|.|..|./*|../*|*/.|*/..|*/./*|*/../*|*//*|*/*/*)
        ;;
      */*)
        if [ -f "$XFSTESTS_DIR/tests/$test_name" ]; then
          printf '%s\n' "$test_name" >> "$SELECTED_TESTS"
          continue
        fi
        ;;
    esac

    echo "Error: unknown xfstests test: $test_name" >&2
    invalid_selector=1
  done < "$REQUESTED_TESTS"

  if [ "$selected_count" -eq 0 ]; then
    echo "$0: CONFORMANCE_TEST_SELECTOR contains no test names" >&2
    exit 2
  fi
  if [ "$invalid_selector" -ne 0 ]; then
    exit 2
  fi

  sort -u -o "$SELECTED_TESTS" "$SELECTED_TESTS"
else
  cat "$REQUESTED_TESTS" > "$SELECTED_TESTS"
  if [ -n "$RUNLIST_FILE" ]; then
    case "$RUNLIST_FILE" in
      */*)
        echo "Run list must be a filename: $RUNLIST_FILE" >&2
        exit 2
        ;;
    esac
    RUNLIST_FILE="$XFSTESTS_FS_DIR/run_list/$RUNLIST_FILE"
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
      printf '%s\n' "$test" >> "$SELECTED_TESTS"
    done < "$RUNLIST_FILE"
  fi
fi

set --
if [ -z "$CONFORMANCE_TEST_SELECTOR" ] && [ -f "$XFSTESTS_BLOCK_LIST" ]; then
    set -- -E "$XFSTESTS_BLOCK_LIST"
fi
while IFS= read -r test_name || [ -n "$test_name" ]; do
    [ -z "$test_name" ] && continue
    set -- "$@" "$test_name"
done < "$SELECTED_TESTS"

./check "$@"
