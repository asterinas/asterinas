#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

TEST_TIME=${1:-60}
TEST_THREADS=${2:-200}

echo "*** Doing sysbench with ${TEST_THREADS} threads for ${TEST_TIME} seconds ***"

/benchmark/bin/sysbench threads \
    --threads=${TEST_THREADS} \
    --thread-yields=100 \
    --thread-locks=4 \
    --time=${TEST_TIME} run 
