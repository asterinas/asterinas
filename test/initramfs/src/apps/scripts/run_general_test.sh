#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

SCRIPT_DIR=/test

for dir in $(find -L "${SCRIPT_DIR}" -mindepth 1 -maxdepth 1 -type d); do
    if [ -x "${dir}/run_test.sh" ]; then
        echo "Running test in $dir"
        (cd "$dir" && ./run_test.sh)
        echo "All test in $dir passed."
    else
        echo "Skipping $dir (no executable TEST_SCRIPT)"
    fi
done

echo "All general tests passed."
