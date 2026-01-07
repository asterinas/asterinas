#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

TEST_TIME=${1:-60}
TEST_THREADS=${2:-4}

echo "*** Doing sysbench CPU test with ${TEST_THREADS} threads for ${TEST_TIME} seconds ***"

/benchmark/bin/sysbench cpu \
    --threads=${TEST_THREADS} \
    --time=${TEST_TIME} \
    --cpu-max-prime=20000 run
    