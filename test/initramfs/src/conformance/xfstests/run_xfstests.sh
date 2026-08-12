#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -eu

# RUNTIME_PATH is substituted by the Nix build.
export PATH=__RUNTIME_PATH__

XFSTESTS_DIR=/opt/xfstests
cd "$XFSTESTS_DIR"

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
        if [ ! -f "$RUNLIST_FILE" ]; then
            echo "Run list file not found: $RUNLIST_FILE" >&2
            exit 2
        fi
        while IFS= read -r test; do
            case "$test" in
                ""|\#*) continue ;;
            esac
            printf '%s\n' "$test" >> "$SELECTED_TESTS"
        done < "$RUNLIST_FILE"
    fi
fi

set --
if [ -z "$CONFORMANCE_TEST_SELECTOR" ] && [ -f "$XFSTESTS_DIR/block.list" ]; then
    set -- -E "$XFSTESTS_DIR/block.list"
fi
while IFS= read -r test_name || [ -n "$test_name" ]; do
    [ -z "$test_name" ] && continue
    set -- "$@" "$test_name"
done < "$SELECTED_TESTS"

TEST_DEV=${XFSTESTS_TEST_DEV:-/dev/vdd}
SCRATCH_DEV=${XFSTESTS_SCRATCH_DEV:-/dev/vde}
export TEST_DEV SCRATCH_DEV

# Mount xfstests images with explicit error checking so a mount failure is not
# silently skipped (which would cause ./check to run against empty directories
# and still print the "all passed" success line).
for entry in "$TEST_DEV:$XFSTESTS_DIR/test:test" "$SCRATCH_DEV:$XFSTESTS_DIR/scratch:scratch"; do
    dev="${entry%%:*}"
    rest="${entry#*:}"
    mnt="${rest%%:*}"
    role="${rest##*:}"
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

./check "$@"
