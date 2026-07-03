#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

LTP_DIR=$(dirname "$0")
TEST_TMP_DIR=${CONFORMANCE_TEST_WORKDIR:-/tmp}
KIRK_TMP_DIR=${KIRK_TMP_DIR:-/tmp}
LOG_FILE=$TEST_TMP_DIR/result.log
JSON_REPORT=$TEST_TMP_DIR/result.json
RESULT=0

# Some test cases require a block device. Select the dedicated LTP device by default.
export LTP_DEV=${LTP_DEV:-/dev/vdc}
export LTP_TIMEOUT_MUL=5
export LTPROOT=$LTP_DIR
export TMPDIR=$TEST_TMP_DIR
export LTP_COLORIZE_OUTPUT=0
export KCONFIG_SKIP_CHECK=1

chmod 1777 "$TEST_TMP_DIR" 2>/dev/null || true
rm -f "$LOG_FILE" "$JSON_REPORT"
KIRK_ARGS="--run-suite syscalls"
if [ "$KIRK_VERBOSE" = "1" ]; then
    KIRK_ARGS="--verbose $KIRK_ARGS"
fi

CREATE_ENTRIES=1 "$LTP_DIR/kirk" --no-colors --tmp-dir "$KIRK_TMP_DIR" \
    --json-report "$JSON_REPORT" $KIRK_ARGS > "$LOG_FILE" 2>&1
if [ $? -ne 0 ]; then
    RESULT=1
fi

cat "$LOG_FILE"
if [ -f "$JSON_REPORT" ]; then
    STATS_BLOCK=$(sed -n '/"stats": {/,/}/p' "$JSON_REPORT")
    if ! echo "$STATS_BLOCK" | grep -q '"failed": 0' ||
       ! echo "$STATS_BLOCK" | grep -q '"broken": 0' ||
       ! echo "$STATS_BLOCK" | grep -q '"warnings": 0'; then
        RESULT=1
    fi
else
    RESULT=1
fi

exit $RESULT
