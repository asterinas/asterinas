#!/bin/bash

# usage: macrometis.sh [linux|asterinas]

SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
BENCH_SCRIPT="$SCRIPT_DIR/bench.sh"
CORTEN_OUTPUT_DIR="$SCRIPT_DIR/../corten_outputs"
mkdir -p "$CORTEN_OUTPUT_DIR"

SYS_NAME=$1

if [ "$SYS_NAME" != "linux" ] && [ "$SYS_NAME" != "aster" ]; then
    echo "Usage: $0 [linux|aster]"
    exit 1
fi

BENCH_OUTPUT_FILE="$CORTEN_OUTPUT_DIR/macrometis_${SYS_NAME}_$(date +%Y%m%d_%H%M%S).log"

THREAD_COUNTS=(1 2 4 8 16 32 64 128 192 256 320 384)
NUM_HOST_CPUS=$(nproc)
for THREAD_COUNT in "${THREAD_COUNTS[@]}"; do
    if [ $THREAD_COUNT -gt $NUM_HOST_CPUS ]; then
        echo "$THREAD_COUNT is greater than the number of host CPUs ($NUM_HOST_CPUS), skipping..."
        continue
    fi
    export NR_CPUS=$THREAD_COUNT
    $BENCH_SCRIPT $SYS_NAME $BENCH_OUTPUT_FILE "/test/corten_benchmetis.sh $THREAD_COUNT"
done
