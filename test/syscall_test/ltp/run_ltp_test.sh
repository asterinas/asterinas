#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

LTP_DIR=$(dirname "$0")
TEST_TMP_DIR=${SYSCALL_TEST_WORKDIR:-/tmp}
LOG_FILE=$TEST_TMP_DIR/result.log
RESULT=0

rm -f $LOG_FILE
CREATE_ENTRIES=1 $LTP_DIR/runltp -f syscalls -p -d $TEST_TMP_DIR -l $LOG_FILE
if [ $? -ne 0 ]; then
    RESULT=1
fi

cat $LOG_FILE
if ! grep -q "Total Failures: 0" $LOG_FILE; then
    RESULT=1
fi

exit $RESULT
