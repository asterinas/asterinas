#!/bin/bash

# usage: macrojvm.sh [linux|asterinas]

SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
BENCH_SCRIPT="$SCRIPT_DIR/bench.sh"
CORTEN_OUTPUT_DIR="$SCRIPT_DIR/../corten_outputs"
mkdir -p "$CORTEN_OUTPUT_DIR"

SYS_NAME=$1

if [ "$SYS_NAME" != "linux" ] && [ "$SYS_NAME" != "aster" ]; then
    echo "Usage: $0 [linux|aster]"
    exit 1
fi

BENCH_OUTPUT_FILE="$CORTEN_OUTPUT_DIR/macrojvm_${SYS_NAME}_$(date +%Y%m%d_%H%M%S).log"

THREAD_COUNTS=(1 2 4 8 16 32 64 128 192 256 320 384)
for THREAD_COUNT in "${THREAD_COUNTS[@]}"; do
    if [ $THREAD_COUNT -eq 320 ]; then
        export NR_CPUS=384
    else
        export NR_CPUS=$THREAD_COUNT
    fi
    export CORTEN_RUN_ARGS="FEATURES=mprotect_async_tlb"
    $BENCH_SCRIPT $SYS_NAME $BENCH_OUTPUT_FILE "/test/corten_benchjvm.sh $THREAD_COUNT"
done
