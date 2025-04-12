#!/bin/bash

# usage: macropsearchy.sh [linux|asterinas] [tc|pt] [aster_breakdown]

SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
BENCH_SCRIPT="$SCRIPT_DIR/bench.sh"
CORTEN_OUTPUT_DIR="$SCRIPT_DIR/../corten_outputs"
mkdir -p "$CORTEN_OUTPUT_DIR"

SYS_NAME=$1
MALLOC=$2

if [ "$SYS_NAME" != "linux" ] && [ "$SYS_NAME" != "aster" ] || [ "$MALLOC" != "tc" ] && [ "$MALLOC" != "pt" ]; then
    echo "Usage: $0 <linux|aster> <tc|pt> [aster_breakdown]"
    exit 1
fi

DO_ASTER_BREAKDOWN=$3
if [ "$SYS_NAME" == "linux" ]; then
    DO_ASTER_BREAKDOWN=""
fi

if [ "$SYS_NAME" == "linux" ]; then
    EXTRA_MNT_CMDS="mount -t devtmpfs devtmpfs /dev; mount -t ext2 /dev/vdb /benchmark/bin/vm_scale_bench_data"
else
    EXTRA_MNT_CMDS="echo 0"
fi

BENCH_OUTPUT_FILE="$CORTEN_OUTPUT_DIR/macropsearchy_${SYS_NAME}_$(date +%Y%m%d_%H%M%S).log"

THREAD_COUNTS=(1 2 4 8 16 32 64 128 192 256 320 384)
NUM_HOST_CPUS=$(nproc)
for THREAD_COUNT in "${THREAD_COUNTS[@]}"; do
    if [ $THREAD_COUNT -gt $NUM_HOST_CPUS ]; then
        echo "$THREAD_COUNT is greater than the number of host CPUs ($NUM_HOST_CPUS), skipping..."
        continue
    fi
    export NR_CPUS=$THREAD_COUNT
    $BENCH_SCRIPT $SYS_NAME $BENCH_OUTPUT_FILE "$EXTRA_MNT_CMDS; /test/corten_benchpsearchy.sh $MALLOC $THREAD_COUNT $DO_ASTER_BREAKDOWN"
done
