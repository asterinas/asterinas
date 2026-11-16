#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -eu

[ "$#" -eq 0 ] || exit 2
. "$(dirname "$0")/common.sh"

EROFS_MATERIALS_DEV=/dev/vdd
EROFS_SCRATCH_DEV=/dev/vde
EROFS_SCRATCH2_DEV=/dev/vdf
export SCRATCH_DEV="$EROFS_SCRATCH_DEV"
export SCRATCH2_DEV="$EROFS_SCRATCH2_DEV"
export EROFS_MATERIALS="$work/materials"
SKIP_EXIT_STATUS=77

require_erofs_device()
{
    if [ ! -b "$1" ]; then
        echo "EROFS test device is missing: $1" >&2
        exit 2
    fi
}

cleanup_runner()
{
    result=$?
    trap - EXIT
    rm -rf "$work"
    exit "$result"
}
trap cleanup_runner EXIT
trap 'exit 1' HUP INT QUIT TERM

select_erofs_tests

require_erofs_device "$EROFS_MATERIALS_DEV"
mkdir -p "$EROFS_MATERIALS"
tar -xf "$EROFS_MATERIALS_DEV" -C "$EROFS_MATERIALS"
rm -rf "$RESULT_DIR"
mkdir -p "$RESULT_DIR/erofs" "$SCRATCH_MNT" "$SCRATCH2_MNT"

failed_count=0
passed_count=0
skipped_count=0
while IFS= read -r test_name; do
    for mount_dir in "$SCRATCH_MNT" "$SCRATCH2_MNT"; do
        if mountpoint -q "$mount_dir"; then
            umount "$mount_dir"
        fi
    done

    require_erofs_device "$EROFS_SCRATCH_DEV"
    case "$test_name" in
        erofs/008|erofs/009|erofs/010|erofs/011)
            require_erofs_device "$EROFS_SCRATCH2_DEV"
            ;;
    esac

    export EROFS_TEST_TMP="$work/tmp-${test_name#erofs/}"
    echo "Running $test_name..."
    set +e
    (cd "$srcdir" && bash "$test_name" --check "$EROFS_MATERIALS/${test_name#erofs/}")
    status=$?
    set -e
    case "$status" in
        0)
            echo "PASS: $test_name"
            passed_count=$((passed_count + 1))
            ;;
        "$SKIP_EXIT_STATUS")
            echo "SKIP: $test_name"
            skipped_count=$((skipped_count + 1))
            ;;
        *)
            echo "FAIL: $test_name (exit $status)" >&2
            failed_count=$((failed_count + 1))
            ;;
    esac
    # A remaining mount means cleanup failed; do not reuse either scratch.
    for mount_dir in "$SCRATCH_MNT" "$SCRATCH2_MNT"; do
        if mountpoint -q "$mount_dir"; then
            echo "EROFS scratch cleanup failed: $mount_dir" >&2
            exit 1
        fi
    done
    for suffix in notrun full out.bad; do
        log="$RESULT_DIR/$test_name.$suffix"
        if [ -s "$log" ]; then
            echo "--- $test_name.$suffix ---"
            cat "$log"
        fi
    done
done <"$SELECTED_TESTS"

echo "EROFS results: $passed_count passed, $skipped_count skipped, $failed_count failed."
[ "$failed_count" -eq 0 ]
