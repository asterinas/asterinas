#!/bin/sh

SCRIPT_DIR=$(dirname "$0")
TEST_TMP_DIR=${SYSCALL_TEST_DIR:-/tmp}
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
        BLOCK=$(sed ':a;N;$!ba;s/\n/:/g' $BLOCKLIST_DIR/$1)
        return 0
    else
        BLOCK=""
        return 1
    fi
}

run_one_test(){
    echo -e "Run Test Case: $1"
    # The gvisor test framework utilizes the "TEST_TMPDIR" environment variable to dictate the directory's location.
    export TEST_TMPDIR=$TEST_TMP_DIR
    ret=0
    if [ -f $TEST_BIN_DIR/$1 ]; then
        rm -rf $TEST_TMP_DIR/*
        get_blocklist_subtests $1
        $TEST_BIN_DIR/$1 --gtest_filter=-$BLOCK
        ret=$?
    else
        echo -e "Warning: $1 test does not exit"
        ret=1
    fi
    echo ""
    return $ret
}

rm -f $FAIL_CASES && touch $FAIL_CASES

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

echo -e "$GREEN$PASSED_TESTS$NC of $GREEN$TESTS$NC test cases are passed."
[ $PASSED_TESTS -ne $TESTS ] && RESULT=1
if [ $TESTS != $PASSED_TESTS ]; then
    echo -e "The $RED$(($TESTS-$PASSED_TESTS))$NC failed test cases in this run are as follows:"
    cat $FAIL_CASES
fi

exit $RESULT
