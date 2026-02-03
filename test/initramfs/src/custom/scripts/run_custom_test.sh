#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

CUSTOM_TEST_DIR=/test
TEST_SCRIPT=run_test.sh

for dir in $(find -L "${CUSTOM_TEST_DIR}" -mindepth 1 -maxdepth 1 -type d); do
    if [ -x "${dir}/${TEST_SCRIPT}" ]; then
        echo "Running test in $dir"
        (cd "$dir" && ./${TEST_SCRIPT})
        echo "All test in $dir passed."
    else
        echo "Skipping $dir (no executable TEST_SCRIPT)"
    fi
done

echo "All custom tests passed."
