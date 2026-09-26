#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

# Run the pjdfstest suite inside Asterinas.

set -u

# RUNTIME_PATH is substituted by the Nix build.
export PATH=__RUNTIME_PATH__

PJDFSTEST_DIR=$(dirname "$0")
SUITE_DIR=$PJDFSTEST_DIR/tests
HELPER=$PJDFSTEST_DIR/pjdfstest
SUITE_CONF=$SUITE_DIR/conf
RUNLIST_FILE=$PJDFSTEST_DIR/runlist
BLOCKLIST_FILE=$PJDFSTEST_DIR/blocklist
TEST_TMP_DIR=${CONFORMANCE_TEST_WORKDIR:-/tmp}
CONFORMANCE_TEST_SELECTOR=${CONFORMANCE_TEST_SELECTOR:-}

WORK_DIR=$TEST_TMP_DIR/pjdfstest
LOG=$WORK_DIR/pjdfstest.log
FAILED_LOG=$WORK_DIR/failed.txt
RESULT=0

REQUESTED_TESTS=$(mktemp)
SELECTED_TESTS=$(mktemp)
BLOCKED_TESTS=$(mktemp)

trap 'rm -f "$REQUESTED_TESTS" "$SELECTED_TESTS" "$BLOCKED_TESTS"' EXIT

if [ ! -x "$HELPER" ]; then
    echo "Error: pjdfstest helper is not executable: $HELPER" >&2
    exit 2
fi
if [ ! -r "$SUITE_CONF" ]; then
    echo "Error: pjdfstest configuration is missing: $SUITE_CONF" >&2
    exit 2
fi

if [ ! -r "$RUNLIST_FILE" ]; then
    echo "Error: pjdfstest runlist is missing: $RUNLIST_FILE" >&2
    exit 2
fi
if [ ! -r "$BLOCKLIST_FILE" ]; then
    echo "Error: pjdfstest blocklist is missing: $BLOCKLIST_FILE" >&2
    exit 2
fi

if [ "$(id -u)" != "0" ]; then
    echo "Error: pjdfstest must be run as root." >&2
    exit 2
fi

rm -rf "$WORK_DIR"
mkdir -p "$WORK_DIR" || exit 2
cd "$WORK_DIR" || exit 2

if [ -n "$CONFORMANCE_TEST_SELECTOR" ]; then
    printf '%s\n' "$CONFORMANCE_TEST_SELECTOR" | tr ',' '\n' > "$REQUESTED_TESTS"
    source_desc="CONFORMANCE_TEST_SELECTOR"
else
    grep -v '^[[:space:]]*#' "$RUNLIST_FILE" | grep -v '^[[:space:]]*$' |
        sed 's/[[:space:]]*$//' > "$REQUESTED_TESTS"
    source_desc="$RUNLIST_FILE"
fi

requested_count=0
invalid_entry=0
while IFS= read -r test_name || [ -n "$test_name" ]; do
    test_name=${test_name#"${test_name%%[![:space:]]*}"}
    test_name=${test_name%"${test_name##*[![:space:]]}"}
    [ -z "$test_name" ] && continue

    requested_count=$((requested_count + 1))
    case "$test_name" in
    /* | -* | . | .. | ./* | ../* | */. | */.. | *//* | */*/*)
        ;;
    */*)
        if [ -f "$SUITE_DIR/$test_name" ]; then
            printf '%s\n' "$test_name" >> "$SELECTED_TESTS"
            continue
        fi
        ;;
    esac

    echo "Error: unknown pjdfstest test case in $source_desc: $test_name" >&2
    invalid_entry=1
done < "$REQUESTED_TESTS"

if [ "$requested_count" -eq 0 ]; then
    echo "Error: $source_desc contains no test names" >&2
    exit 2
fi
if [ "$invalid_entry" -ne 0 ]; then
    echo "Error: $source_desc contains invalid entries" >&2
    exit 2
fi

sort -u -o "$SELECTED_TESTS" "$SELECTED_TESTS"

if [ -z "$CONFORMANCE_TEST_SELECTOR" ]; then
    grep -v '^[[:space:]]*#' "$BLOCKLIST_FILE" | grep -v '^[[:space:]]*$' |
        sed 's/[[:space:]]*$//' | sort -u > "$BLOCKED_TESTS"
    if [ -s "$BLOCKED_TESTS" ]; then
        comm -23 "$SELECTED_TESTS" "$BLOCKED_TESTS" > "$SELECTED_TESTS.tmp"
        mv "$SELECTED_TESTS.tmp" "$SELECTED_TESTS"
    fi
fi

# `prove` needs absolute paths
set --
while IFS= read -r test_name || [ -n "$test_name" ]; do
    set -- "$@" "$SUITE_DIR/$test_name"
done < "$SELECTED_TESTS"

TEST_COUNT=$(wc -l < "$SELECTED_TESTS" | tr -d ' ')

if [ "$TEST_COUNT" -eq 0 ]; then
    echo "Error: no pjdfstest test cases selected." >&2
    exit 2
fi

echo "Running ${TEST_COUNT} pjdfstest test cases on ${TEST_TMP_DIR}..."

prove -v "$@" 2>&1 | tee "$LOG"
if grep -q '^Result: FAIL' "$LOG"; then
    PROVE_STATUS=1
else
    PROVE_STATUS=0
fi

grep '^not ok' "$LOG" | grep -v '# TODO' > "$FAILED_LOG" || true
FAILED_COUNT=$(wc -l < "$FAILED_LOG" | tr -d ' ')

echo ""
if [ "$FAILED_COUNT" -ne 0 ]; then
    echo "Failed assertions:"
    cat "$FAILED_LOG"
    echo ""
fi
sed -n '/^Test Summary Report/,$p' "$LOG"

if [ "$PROVE_STATUS" -ne 0 ]; then
    RESULT=1
fi
if [ "$FAILED_COUNT" -ne 0 ]; then
    RESULT=1
fi

# A run that reports no tests at all means the suite was mispackaged, which
# `prove` itself treats as a failure by exiting non-zero; check explicitly so
# the reason is visible.
if ! grep -q '^Files=' "$LOG"; then
    echo "Error: pjdfstest produced no test summary; see $LOG" >&2
    echo "--- first 30 lines of $LOG ---" >&2
    head -30 "$LOG" >&2
    echo "--- last 10 lines of $LOG ---" >&2
    tail -10 "$LOG" >&2
    RESULT=1
fi

exit $RESULT
