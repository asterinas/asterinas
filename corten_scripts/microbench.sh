#!/bin/bash

# usage: microbench.sh [linux|aster]

SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
BENCH_SCRIPT="$SCRIPT_DIR/bench.sh"
CORTEN_OUTPUT_DIR="$SCRIPT_DIR/../corten_outputs"
mkdir -p "$CORTEN_OUTPUT_DIR"

SYS_NAME=$1

if [ "$SYS_NAME" != "linux" ] && [ "$SYS_NAME" != "aster" ]; then
    echo "Usage: $0 [linux|aster]"
    exit 1
fi

BENCH_OUTPUT_FILE="$CORTEN_OUTPUT_DIR/microbench_${SYS_NAME}_$(date +%Y%m%d_%H%M%S).log"

THREAD_COUNTS=(1 2 4 8 16 32 64 128 192 256 320 384)
NUM_HOST_CPUS=$(nproc)
for THREAD_COUNT in "${THREAD_COUNTS[@]}"; do
    if [ $THREAD_COUNT -gt $NUM_HOST_CPUS ]; then
        echo "$THREAD_COUNT is greater than the number of host CPUs ($NUM_HOST_CPUS), skipping..."
        continue
    fi
    export NR_CPUS=$THREAD_COUNT
    $BENCH_SCRIPT $SYS_NAME $BENCH_OUTPUT_FILE "/test/scale/mmap unfixed $THREAD_COUNT"
    $BENCH_SCRIPT $SYS_NAME $BENCH_OUTPUT_FILE "/test/scale/mmap fixed 1 $THREAD_COUNT"
    $BENCH_SCRIPT $SYS_NAME $BENCH_OUTPUT_FILE "/test/scale/mmap_pf unfixed $THREAD_COUNT"
    $BENCH_SCRIPT $SYS_NAME $BENCH_OUTPUT_FILE "/test/scale/mmap_pf fixed 1 $THREAD_COUNT"
    $BENCH_SCRIPT $SYS_NAME $BENCH_OUTPUT_FILE "/test/scale/pf 0 $THREAD_COUNT"
    $BENCH_SCRIPT $SYS_NAME $BENCH_OUTPUT_FILE "/test/scale/pf 1 $THREAD_COUNT"
    $BENCH_SCRIPT $SYS_NAME $BENCH_OUTPUT_FILE "/test/scale/munmap_virt 0 $THREAD_COUNT"
    $BENCH_SCRIPT $SYS_NAME $BENCH_OUTPUT_FILE "/test/scale/munmap_virt 1 $THREAD_COUNT"
    $BENCH_SCRIPT $SYS_NAME $BENCH_OUTPUT_FILE "/test/scale/munmap 0 $THREAD_COUNT"
    $BENCH_SCRIPT $SYS_NAME $BENCH_OUTPUT_FILE "/test/scale/munmap 1 $THREAD_COUNT"
done
