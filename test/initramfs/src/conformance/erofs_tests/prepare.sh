#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -eu

[ "$#" -ge 1 ] && [ "$#" -le 2 ] || exit 2
CONFORMANCE_TEST_SELECTOR=${2:-}
. "$(dirname "$0")/common.sh"

cleanup_prepare()
{
    result=$?
    trap - EXIT
    # Failed cleanup can leave source mounts inside work. Keep it intact.
    if [ "$result" -ne 0 ]; then
        echo "EROFS preparation failed; retained work directory: $work" >&2
    else
        rm -rf "$work"
    fi
    exit "$result"
}
trap cleanup_prepare EXIT
trap 'exit 1' HUP INT QUIT TERM

output=$(realpath -m "$1")
select_erofs_tests

export RESULT_DIR="$work/results"
export SCRATCH_DEV="$work/scratch.img"
export SCRATCH2_DEV="$work/scratch2.img"
export SCRATCH_MNT="$work/scratch"
export SCRATCH2_MNT="$work/scratch2"
mkdir -p "$work/materials" "$RESULT_DIR/erofs" "$SCRATCH_MNT" "$SCRATCH2_MNT"
while IFS= read -r test_name; do
    seq=${test_name#erofs/}
    export EROFS_TEST_TMP="$work/tmp-$seq"
    case_dir="$work/materials/$seq"
    mkdir -p "$case_dir"
    echo "Preparing $test_name..."
    set +e
    (cd "$srcdir" && bash "$test_name" --prepare "$case_dir")
    status=$?
    set -e
    log="$RESULT_DIR/$test_name.full"
    if [ -f "$log" ]; then
        cp "$log" "$case_dir/prepare.full"
    fi
    if [ "$status" -ne 0 ]; then
        for suffix in full notrun; do
            log="$RESULT_DIR/$test_name.$suffix"
            [ ! -s "$log" ] || cat "$log" >&2
        done
        exit "$status"
    fi
done <"$SELECTED_TESTS"
mkdir -p "$(dirname "$output")"
tar --format=gnu -cf "$output" -C "$work/materials" .
