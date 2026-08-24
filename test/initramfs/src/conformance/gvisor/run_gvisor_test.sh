#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

SCRIPT_DIR=$(dirname "$0")
TEST_TMP_DIR=${CONFORMANCE_TEST_WORKDIR:-/tmp}
TEST_BIN_DIR=$SCRIPT_DIR/tests
BLOCKLIST_DIR=$SCRIPT_DIR/blocklists
FAIL_CASES=$SCRIPT_DIR/fail_cases
BLOCK=""
TESTS=0
PASSED_TESTS=0
RESULT=0
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m'

# When a selector is set, run only the selected test binaries and ignore the blocklist.
CONFORMANCE_TEST_SELECTOR=${CONFORMANCE_TEST_SELECTOR:-}
CONFORMANCE_TEST_GVISOR_FILTER=${CONFORMANCE_TEST_GVISOR_FILTER:-}
SELECTED_TESTS=$(mktemp "$SCRIPT_DIR/selected_tests.XXXXXX")

trap 'rm -f "$SELECTED_TESTS"' EXIT

get_blocklist_subtests(){
    if [ -f $BLOCKLIST_DIR/$1 ]; then
        BLOCK=$(grep -v '^#' $BLOCKLIST_DIR/$1 | tr '\n' ':')
    else
        BLOCK=""
    fi

    remaining_blocklists="${CONFORMANCE_TEST_EXTRA_BLOCKLISTS:-},"
    while [ -n "$remaining_blocklists" ]; do
        extra_dir=${remaining_blocklists%%,*}
        remaining_blocklists=${remaining_blocklists#*,}
        [ -z "$extra_dir" ] && continue

        if [ -f "$SCRIPT_DIR/$extra_dir/$1" ]; then
            BLOCK="${BLOCK}:$(grep -v '^#' "$SCRIPT_DIR/$extra_dir/$1" | tr '\n' ':')"
        fi
    done

    return 0
}

run_one_test(){
    echo -e "Run Test Case: $1"
    # The gvisor test framework utilizes the "TEST_TMPDIR" environment variable to dictate the directory's location.
    export TEST_TMPDIR=$TEST_TMP_DIR
    ret=0
    if [ -x "$TEST_BIN_DIR/$1" ]; then
        if [ -n "$CONFORMANCE_TEST_SELECTOR" ]; then
            gtest_filter=${CONFORMANCE_TEST_GVISOR_FILTER:-*}
        else
            get_blocklist_subtests $1
            gtest_filter="-$BLOCK"
        fi
        cd "$TEST_BIN_DIR" && "./$1" --gtest_filter="$gtest_filter"
        ret=$?
        #After executing the test, it is necessary to clean the directory to ensure no residual data remains
        rm -rf $TEST_TMP_DIR/*
    else
        echo "Error: gVisor test is not executable: $1" >&2
        ret=1
    fi
    echo ""
    return $ret
}

if [ -n "$CONFORMANCE_TEST_SELECTOR" ]; then
    REQUESTED_TESTS=$(mktemp "$SCRIPT_DIR/requested_tests.XXXXXX")
    trap 'rm -f "$SELECTED_TESTS" "$REQUESTED_TESTS"' EXIT
    printf '%s\n' "$CONFORMANCE_TEST_SELECTOR" | tr ',' '\n' > "$REQUESTED_TESTS"

    selected_count=0
    invalid_selector=0
    while IFS= read -r test_name || [ -n "$test_name" ]; do
        test_name=${test_name#"${test_name%%[![:space:]]*}"}
        test_name=${test_name%"${test_name##*[![:space:]]}"}
        [ -z "$test_name" ] && continue
        test_name=$(basename "$test_name")

        selected_count=$((selected_count + 1))
        case "$test_name" in
            *_test)
                if [ -x "$TEST_BIN_DIR/$test_name" ]; then
                    printf '%s\n' "$test_name" >> "$SELECTED_TESTS"
                    continue
                fi
                ;;
        esac

        echo "Error: unknown gVisor test: $test_name" >&2
        invalid_selector=1
    done < "$REQUESTED_TESTS"

    if [ "$selected_count" -eq 0 ]; then
        echo "Error: CONFORMANCE_TEST_SELECTOR contains no test names" >&2
        exit 2
    fi
    if [ "$invalid_selector" -ne 0 ]; then
        exit 2
    fi

    sort -u -o "$SELECTED_TESTS" "$SELECTED_TESTS"
else
    find "$TEST_BIN_DIR" -maxdepth 1 -type f -name '*_test' \
        -exec basename {} \; | sort -u > "$SELECTED_TESTS"
fi

rm -f "$FAIL_CASES" && touch "$FAIL_CASES"
rm -rf "$TEST_TMP_DIR"/*

while IFS= read -r test_name; do
    run_one_test "$test_name"
    if [ $? -eq 0 ] && PASSED_TESTS=$((PASSED_TESTS+1));then
        TESTS=$((TESTS+1))
    else
        echo "$test_name" >> "$FAIL_CASES"
        TESTS=$((TESTS+1))
    fi
done < "$SELECTED_TESTS"

echo -e "$GREEN$PASSED_TESTS$NC of $GREEN$TESTS$NC test cases passed."
[ $PASSED_TESTS -ne $TESTS ] && RESULT=1
if [ $TESTS != $PASSED_TESTS ]; then
    echo -e "The $RED$(($TESTS-$PASSED_TESTS))$NC failed test cases are as follows:"
    cat $FAIL_CASES
fi

exit $RESULT
