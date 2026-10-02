#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

# Run the pjdfstest suite inside Asterinas.

set -u

# RUNTIME_PATH is substituted by the Nix build.
export PATH=__RUNTIME_PATH__

PJDFSTEST_DIR=$(realpath -ms "$(dirname "$0")")
SUITE_DIR=$PJDFSTEST_DIR/tests
HELPER=$PJDFSTEST_DIR/pjdfstest
SUITE_CONF=$SUITE_DIR/conf
CONFORMANCE_TEST_SELECTOR=${CONFORMANCE_TEST_SELECTOR:-}

CONFORMANCE_TEST_WORKDIR=$(realpath -ms "${CONFORMANCE_TEST_WORKDIR:-/tmp}")
export CONFORMANCE_TEST_WORKDIR
TEST_TMP_DIR=$CONFORMANCE_TEST_WORKDIR

# The suite writes every path relative to the working directory, so the
# filesystem under test is whichever filesystem that directory lives on. Both
# the runlist and the blocklist differ per filesystem, so this has to be known
# before either can be located.
#
# `conf` already derives that filesystem -- `tests/misc.sh` sources it for the
# same reason -- so the runner sources it too rather than repeating the probe.
# It names the result in `fs`, which is also the name of the per-filesystem
# directories below.
if [ ! -r "$SUITE_CONF" ]; then
    echo "Error: pjdfstest configuration is missing: $SUITE_CONF" >&2
    exit 2
fi
. "$SUITE_CONF"

PJDFSTEST_FS=${fs:-}
if [ -z "$PJDFSTEST_FS" ]; then
    echo "Error: cannot identify the filesystem backing $TEST_TMP_DIR." >&2
    exit 2
fi

RUNLIST_FILE=$PJDFSTEST_DIR/$PJDFSTEST_FS/runlist
BLOCKLIST_FILE=$PJDFSTEST_DIR/$PJDFSTEST_FS/blocklist

# Per-case wall-clock limit. A case that wedges the kernel never returns on its
# own, so the guest would otherwise spin until CI kills the whole job.
TEST_TIMEOUT=${PJDFSTEST_TEST_TIMEOUT:-300}

# Resolve WORK_DIR to an absolute path before deriving LOG and FAILED_LOG from
# it. Both are used after the runner has `cd`-ed into WORK_DIR, so a relative
# value would make them resolve one directory too deep, leaving the run with no
# log and no failed-assertion list while still exiting successfully.
# `-m` tolerates the not-yet-created work directory, and `-s` keeps symbolic
# links unresolved so the path still matches the mount points in /proc/mounts
# that identify the filesystem under test.
WORK_DIR=$(realpath -ms "$TEST_TMP_DIR/pjdfstest")
LOG=$WORK_DIR/pjdfstest.log
FAILED_LOG=$WORK_DIR/failed.txt
RESULT=0

# Holds the generated per-case wrappers. It lives inside WORK_DIR, which is
# wiped and recreated below, so that it never appears on a filesystem the cases
# inspect: several of them (`chown/00.t`, `rename/09.t`) enumerate the directory
# they run in. It cannot live in $SUITE_DIR, which is packaged read-only.
#
# `tests/misc.sh` locates the helper by walking up from the directory of the
# script that was executed -- which is now the wrapper, not the case -- looking
# for an executable named `pjdfstest`. The walk stops as soon as it finds one,
# so the wrapper directory carries its own name for the helper.
WRAPPER_DIR=$WORK_DIR/.wrappers

REQUESTED_TESTS=$(mktemp)
SELECTED_TESTS=$(mktemp)
BLOCKED_TESTS=$(mktemp)

trap 'rm -f "$REQUESTED_TESTS" "$SELECTED_TESTS" "$BLOCKED_TESTS"' EXIT

if [ ! -x "$HELPER" ]; then
    echo "Error: pjdfstest helper is not executable: $HELPER" >&2
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

mkdir -p "$WRAPPER_DIR" || exit 2
ln -sf "$HELPER" "$WRAPPER_DIR/pjdfstest" || exit 2

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

# `prove` takes test files, not commands, so the timeout cannot be passed to it
# directly -- and a case that wedges the kernel never returns on its own. Each
# selected case therefore gets a one-line wrapper that imposes the limit, and
# `prove` runs the wrappers. Without this, a single hung case leaves the guest
# spinning until CI kills the whole job and no later case ever reports.
set --
while IFS= read -r test_name || [ -n "$test_name" ]; do
    # Mirroring the case path keeps the name `prove` reports recognizable:
    # `<wrapper dir>/chmod/00.t` still contains `chmod/00.t`, which is what
    # someone greps the log for.
    wrapper=$WRAPPER_DIR/$test_name
    mkdir -p "$(dirname "$wrapper")"
    # `--foreground` is deliberately absent: it would keep the case in the
    # runner's process group, so only the case itself would be signalled and any
    # process it spawned would survive holding the pipe open, leaving `prove`
    # blocked forever. Running the case in its own process group lets the whole
    # tree be killed. `--kill-after` escalates to SIGKILL when a process ignores
    # SIGTERM.
    {
        printf '#!/bin/sh\n'
        printf 'exec timeout --kill-after=10 %s %s\n' \
            "$TEST_TIMEOUT" "$SUITE_DIR/$test_name"
    } > "$wrapper"
    chmod +x "$wrapper"
    set -- "$@" "$wrapper"
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

# A case killed by the timeout produces no `not ok` line at all -- `prove`
# reports it as `Dubious, test returned 124` and a non-zero exit status. Without
# this it would inflate the failure count that `prove` reports while staying
# absent from the list of what actually failed.
TIMED_OUT_LOG=$WORK_DIR/timed_out.txt
grep -E '^.*exited 124\)' "$LOG" | sed 's/ (Wstat.*//' | sort -u > "$TIMED_OUT_LOG" || true
TIMED_OUT_COUNT=$(wc -l < "$TIMED_OUT_LOG" | tr -d ' ')

echo ""
if [ "$FAILED_COUNT" -ne 0 ]; then
    echo "Failed assertions:"
    cat "$FAILED_LOG"
    echo ""
fi
if [ "$TIMED_OUT_COUNT" -ne 0 ]; then
    echo "Test cases that exceeded the ${TEST_TIMEOUT}s limit:"
    cat "$TIMED_OUT_LOG"
    echo ""
fi
sed -n '/^Test Summary Report/,$p' "$LOG"

if [ "$PROVE_STATUS" -ne 0 ]; then
    RESULT=1
fi
if [ "$FAILED_COUNT" -ne 0 ]; then
    RESULT=1
fi
if [ "$TIMED_OUT_COUNT" -ne 0 ]; then
    RESULT=1
fi

# A run that reports no tests at all means the suite was mispackaged, which
# `prove` itself treats as a failure by exiting non-zero; check explicitly so
# the reason is visible.
if ! grep -q -e '^Files=' -e '^Result: ' "$LOG"; then
    echo "Error: pjdfstest produced no test summary; see $LOG" >&2
    echo "--- first 30 lines of $LOG ---" >&2
    head -30 "$LOG" >&2
    echo "--- last 10 lines of $LOG ---" >&2
    tail -10 "$LOG" >&2
    RESULT=1
fi

exit $RESULT
