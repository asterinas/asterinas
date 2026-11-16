#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

# Shared environment and selection for the host and guest entry points.
export PATH=__RUNTIME_PATH__
export LC_ALL=C
umask 022

EROFS_DIR=$(cd "$(dirname "$0")" && pwd)
CONFORMANCE_TEST_WORKDIR=${CONFORMANCE_TEST_WORKDIR:-/tmp}
CONFORMANCE_TEST_SELECTOR=${CONFORMANCE_TEST_SELECTOR:-}
case "$CONFORMANCE_TEST_WORKDIR" in
    /*) ;;
    *)
        echo "EROFS work directory must be an absolute path: $CONFORMANCE_TEST_WORKDIR" >&2
        exit 2
        ;;
esac
mkdir -p "$CONFORMANCE_TEST_WORKDIR"
CONFORMANCE_TEST_WORKDIR=$(readlink -f "$CONFORMANCE_TEST_WORKDIR")
# Upstream helpers interpolate these paths into eval commands.
case "$CONFORMANCE_TEST_WORKDIR" in
    *[!a-zA-Z0-9_./-]*)
        echo "EROFS work directory contains unsupported characters: $CONFORMANCE_TEST_WORKDIR" >&2
        exit 2
        ;;
esac
if [ "$CONFORMANCE_TEST_WORKDIR" = / ]; then
    echo "EROFS work directory must not be the root directory" >&2
    exit 2
fi

export FSTYP=erofs
export EROFS_PHASE_HELPER="$EROFS_DIR/phase.sh"
export srcdir="$EROFS_DIR/upstream/tests"
export RESULT_DIR="$CONFORMANCE_TEST_WORKDIR/erofs-results"
export SCRATCH_MNT="$CONFORMANCE_TEST_WORKDIR/erofstest/scratch"
export SCRATCH2_MNT="$CONFORMANCE_TEST_WORKDIR/erofstest/scratch2"
export MKFS_EROFS_PROG="$EROFS_DIR/bin/mkfs.erofs"
export FSCK_EROFS_PROG="$EROFS_DIR/bin/fsck.erofs"
export DUMP_EROFS_PROG="$EROFS_DIR/bin/dump.erofs"
export FSSUM_PROG="$EROFS_DIR/bin/fssum"
export EROFSFUSE_PROG="$EROFS_DIR/bin/erofsfuse"
export MKFS_OPTIONS=-b4096
export lz4_on=1 lz4hc_on=1 lzma_on=1 deflate_on=1 zstd_on=1

work=$(mktemp -d "$CONFORMANCE_TEST_WORKDIR/erofs-run.XXXXXX")
export TMPDIR="$work"

select_erofs_tests()
{
    REQUESTED_TESTS="$work/requested"
    SELECTED_TESTS="$work/selected"
    SUPPORTED_TESTS="$work/supported"
    touch "$SELECTED_TESTS" "$SUPPORTED_TESTS"

    RUNLIST_FILE="$EROFS_DIR/run_list/short.list"
    while IFS= read -r test_name || [ -n "$test_name" ]; do
        test_name=${test_name%%#*}
        test_name=${test_name#"${test_name%%[![:space:]]*}"}
        test_name=${test_name%"${test_name##*[![:space:]]}"}
        [ -z "$test_name" ] && continue
        printf '%s\n' "$test_name" >>"$SUPPORTED_TESTS"
    done <"$RUNLIST_FILE"

    if [ -n "$CONFORMANCE_TEST_SELECTOR" ]; then
        printf '%s\n' "$CONFORMANCE_TEST_SELECTOR" | tr ',' '\n' >"$REQUESTED_TESTS"
    else
        cp "$SUPPORTED_TESTS" "$REQUESTED_TESTS"
    fi

    selected_count=0
    has_invalid_selector=0
    while IFS= read -r test_name || [ -n "$test_name" ]; do
        test_name=${test_name#"${test_name%%[![:space:]]*}"}
        test_name=${test_name%"${test_name##*[![:space:]]}"}
        [ -z "$test_name" ] && continue
        case "$test_name" in
            [0-9][0-9][0-9]) test_name="erofs/$test_name" ;;
        esac
        case "$test_name" in
            erofs/[0-9][0-9][0-9]) ;;
            *)
                echo "invalid EROFS test selector: $test_name" >&2
                has_invalid_selector=1
                continue
                ;;
        esac
        if [ ! -f "$EROFS_DIR/upstream/tests/$test_name" ]; then
            echo "unknown EROFS test: $test_name" >&2
            has_invalid_selector=1
            continue
        fi
        if ! grep -Fxq "$test_name" "$SUPPORTED_TESTS"; then
            echo "unsupported EROFS test (not in run_list/short.list): $test_name" >&2
            has_invalid_selector=1
            continue
        fi
        printf '%s\n' "$test_name" >>"$SELECTED_TESTS"
        selected_count=$((selected_count + 1))
    done <"$REQUESTED_TESTS"

    if [ -n "$CONFORMANCE_TEST_SELECTOR" ]; then
        sort -u -o "$SELECTED_TESTS" "$SELECTED_TESTS"
    fi

    if [ "$selected_count" -eq 0 ] || [ "$has_invalid_selector" -ne 0 ]; then
        exit 2
    fi
}
