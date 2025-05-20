#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

SCRIPT_DIR=$(dirname "$0")
TEST_TMP_DIR=${SYSCALL_TEST_WORKDIR:-/tmp}
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

get_blocklist_subtests(){
    if [ -f $BLOCKLIST_DIR/$1 ]; then
        BLOCK=$(grep -v '^#' $BLOCKLIST_DIR/$1 | tr '\n' ':')
    else
        BLOCK=""
    fi

    for extra_dir in $EXTRA_BLOCKLISTS_DIRS ; do
        if [ -f $SCRIPT_DIR/$extra_dir/$1 ]; then
            BLOCK="${BLOCK}:$(grep -v '^#' $SCRIPT_DIR/$extra_dir/$1 | tr '\n' ':')"
        fi
    done

    return 0
}

run_one_test(){
    echo -e "Run Test Case: $1"
    # The gvisor test framework utilizes the "TEST_TMPDIR" environment variable to dictate the directory's location.
    export TEST_TMPDIR=$TEST_TMP_DIR
    ret=0
    if [ -f $TEST_BIN_DIR/$1 ]; then
        get_blocklist_subtests $1
        cd $TEST_BIN_DIR && ./$1 --gtest_filter=-$BLOCK
        ret=$?
        #After executing the test, it is necessary to clean the directory to ensure no residual data remains
        rm -rf $TEST_TMP_DIR/*
    else
        echo -e "Warning: $1 test does not exit"
        ret=1
    fi
    echo ""
    return $ret
}

rm -f $FAIL_CASES && touch $FAIL_CASES
rm -rf $TEST_TMP_DIR/*

for syscall_test in $(find $TEST_BIN_DIR/. -name \*_test) ; do
    test_name=$(basename "$syscall_test")
    run_one_test $test_name
    if [ $? -eq 0 ] && PASSED_TESTS=$((PASSED_TESTS+1));then
        TESTS=$((TESTS+1))
    else
        echo -e "$test_name" >> $FAIL_CASES
        TESTS=$((TESTS+1))
    fi
done

echo -e "$GREEN$PASSED_TESTS$NC of $GREEN$TESTS$NC test cases passed."
[ $PASSED_TESTS -ne $TESTS ] && RESULT=1
if [ $TESTS != $PASSED_TESTS ]; then
    echo -e "The $RED$(($TESTS-$PASSED_TESTS))$NC failed test cases are as follows:"
    cat $FAIL_CASES
fi

exit $RESULT
