#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

echo "=== Kselftest Runner Started ==="

KSELFTEST_DIR=$(dirname "$0")
DEFAULT_BLOCKLISTS="$KSELFTEST_DIR/blocklists"
COMBINED_BLOCKLISTS=$(mktemp)
RESOLVED_BLOCKLISTS=$(mktemp)
AVAILABLE_TESTS=$(mktemp)
FAILED_TESTS=$(mktemp)

trap 'rm -f "$COMBINED_BLOCKLISTS" "$RESOLVED_BLOCKLISTS" "$AVAILABLE_TESTS" "$FAILED_TESTS"' EXIT

# The `kselftest-list.txt` is a generated file produced by the kselftest build.
#
# Each line describes a single kselftest case in the form:
#     <test-dir>:<test-command>
#
# Example lines (matching enabled TARGETS in kselftest.nix):
#   exec:binfmt_script.py
#   timers:posix_timers
#   vDSO:vdso_test_abi
TESTS="$KSELFTEST_DIR/kselftest-list.txt"
if [ ! -r "$TESTS" ]; then
    echo "$0: missing $TESTS; kselftest build is broken" >&2
    exit 1
else
    available_count=$(grep -cve '^$' "$TESTS")
    echo "Found $available_count available test cases"
fi

if [ ! -r "$DEFAULT_BLOCKLISTS" ]; then
    echo "$0: missing $DEFAULT_BLOCKLISTS; kselftest blocklist is broken" >&2
    exit 1
fi

cat "$DEFAULT_BLOCKLISTS" > "$COMBINED_BLOCKLISTS"
for extra_file in $EXTRA_BLOCKLISTS ; do
    extra_blocklists="$KSELFTEST_DIR/$extra_file"
    if [ -r "$extra_blocklists" ]; then
        printf '\n' >> "$COMBINED_BLOCKLISTS"
        cat "$extra_blocklists" >> "$COMBINED_BLOCKLISTS"
    else
        echo "Warning: extra blocklist not found: $extra_blocklists" >&2
    fi
done

echo "Processing blocklists..."
while IFS= read -r line || [ -n "$line" ]; do
    line=${line#"${line%%[![:space:]]*}"}
    line=${line%"${line##*[![:space:]]}"}
    case "$line" in
        "" | "#"*)
            continue ;;
        *:*)
            dir="${line%%:*}"
            command="${line#*:}"
            if [ "$command" = "*" ]; then
                awk -F: -v d="$dir" '$1 == d { print }' "$TESTS" >> "$RESOLVED_BLOCKLISTS"
            else
                printf '%s\n' "$line" >> "$RESOLVED_BLOCKLISTS"
            fi
            ;;
        *)
            echo "Error: Invalid format in blocklist: $line" >&2
            exit 1
            ;;
    esac
done < "$COMBINED_BLOCKLISTS"

sort -u -o "$RESOLVED_BLOCKLISTS" "$RESOLVED_BLOCKLISTS"
if [ -s "$RESOLVED_BLOCKLISTS" ]; then
    blocked_count=$(wc -l < "$RESOLVED_BLOCKLISTS")
    grep -vxFf "$RESOLVED_BLOCKLISTS" "$TESTS" | grep -v '^$' > "$AVAILABLE_TESTS"
else
    blocked_count=0
    grep -v '^$' "$TESTS" > "$AVAILABLE_TESTS"
fi
echo "Total blocklist entries processed: $blocked_count"

run_count=$(wc -l < "$AVAILABLE_TESTS")
if [ "$run_count" -eq 0 ]; then
    echo "$0: no test cases survived blocklist filtering" >&2
    exit 1
fi
echo "Test cases to be executed: $run_count"

echo "================================"

GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m'
total_tests=$run_count
passed_tests=0
failed_tests=0

if ! command -v timeout >/dev/null 2>&1; then
    echo "$0: missing timeout command" >&2
    exit 1
fi

KSELFTEST_TIMEOUT=${KSELFTEST_TIMEOUT:-300}
run_test() {
    test_dir=$1
    shift

    (
        cd "$test_dir" &&
        timeout "$KSELFTEST_TIMEOUT" "$@"
    )
}

# Use a dedicated subdirectory under the selected workdir.
TEST_WORKDIR="${CONFORMANCE_TEST_WORKDIR:-/tmp}/kselftest"
rm -rf "$TEST_WORKDIR"
mkdir -p "$TEST_WORKDIR"
if [ ! -d "$TEST_WORKDIR" ]; then
    echo "$0: failed to create TEST_WORKDIR: $TEST_WORKDIR" >&2
    exit 1
fi

dirs=$(cut -d: -f1 "$AVAILABLE_TESTS" | sort -u)
for dir in $dirs ; do
    TEST_DIR="$TEST_WORKDIR/$dir"
    echo "Running tests in dir: $TEST_DIR"
    if ! cp -rL "$KSELFTEST_DIR/$dir" "$TEST_DIR"; then
        echo "$0: failed to copy kselftest dir: $KSELFTEST_DIR/$dir" >&2
        exit 1
    fi

    commands=$(awk -F: -v d="$dir" '$1 == d { sub(/^[^:]*:/, ""); print }' "$AVAILABLE_TESTS")
    for command in $commands ; do
        echo "[ PROCESS  ]: $dir:$command"
        bin="$TEST_DIR/$command"
        failure_reason=

        if [ ! -e "$bin" ]; then
            exit_code=127
            failure_reason="missing"
        elif [ -x "$bin" ]; then
            run_test "$TEST_DIR" "./$command"
            exit_code=$?
        else
            case "$command" in
                *.py)
                    if command -v python3 >/dev/null 2>&1; then
                        run_test "$TEST_DIR" python3 "./$command"
                        exit_code=$?
                    else
                        exit_code=127
                        failure_reason="missing interpreter: python3"
                    fi
                    ;;
                *.sh)
                    run_test "$TEST_DIR" sh "./$command"
                    exit_code=$?
                    ;;
                *)
                    exit_code=126
                    failure_reason="not executable"
                    ;;
            esac
        fi

        if [ "$exit_code" -eq 124 ]; then
            failure_reason="timed out after ${KSELFTEST_TIMEOUT}s"
        fi

        if [ "$exit_code" -eq 0 ]; then
            printf '[  %bPASSED%b  ]: %s:%s\n' "$GREEN" "$NC" "$dir" "$command"
            passed_tests=$((passed_tests + 1))
        else
            if [ -n "$failure_reason" ]; then
                printf '[  %bFAILED%b  ]: %s:%s (%s)\n' \
                    "$RED" "$NC" "$dir" "$command" "$failure_reason"
            else
                printf '[  %bFAILED%b  ]: %s:%s (exit code: %s)\n' \
                    "$RED" "$NC" "$dir" "$command" "$exit_code"
            fi
            failed_tests=$((failed_tests + 1))
            printf '%s:%s\n' "$dir" "$command" >> "$FAILED_TESTS"
        fi
    done
done

echo ""
echo "=========== Summary ============"
printf '%b%s%b of %b%s%b test cases passed.\n' \
    "$GREEN" "$passed_tests" "$NC" "$GREEN" "$total_tests" "$NC"
if [ "$failed_tests" -gt 0 ]; then
    printf 'The %b%s%b failed test cases are as follows:\n' \
        "$RED" "$failed_tests" "$NC"
    cat "$FAILED_TESTS"
    exit 1
else
    echo ""
    echo "All kselftest tests passed."
    exit 0
fi
