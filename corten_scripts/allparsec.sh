#!/bin/bash

# usage: macroparsec.sh [linux|asterinas] [aster_breakdown]

SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
BENCH_SCRIPT="$SCRIPT_DIR/bench.sh"
CORTEN_OUTPUT_DIR="$SCRIPT_DIR/../corten_outputs"
mkdir -p "$CORTEN_OUTPUT_DIR"

SYS_NAME=$1

if [ "$SYS_NAME" != "linux" ] && [ "$SYS_NAME" != "aster" ]; then
    echo "Usage: $0 [linux|aster]"
    exit 1
fi

DO_ASTER_BREAKDOWN=$2
if [ "$SYS_NAME" == "linux" ]; then
    DO_ASTER_BREAKDOWN=""
fi

if [ "$SYS_NAME" == "linux" ]; then
    EXTRA_MNT_CMDS="mount -t devtmpfs devtmpfs /dev; mount -t ext2 /dev/vdb /benchmark/bin/vm_scale_bench_data"
else
    EXTRA_MNT_CMDS="echo 0"
fi

PARSEC_BENCHES=(canneal dedup streamcluster blackscholes bodytrack facesim ferret fluidanimate freqmine vips x264 swaptions)

BENCH_OUTPUT_FILE="$CORTEN_OUTPUT_DIR/macroparsec_${SYS_NAME}_$(date +%Y%m%d_%H%M%S).log"

THREAD_COUNTS=(1 8)
for THREAD_COUNT in "${THREAD_COUNTS[@]}"; do
    export NR_CPUS=$THREAD_COUNT
    for BENCH in "${PARSEC_BENCHES[@]}"; do
        $BENCH_SCRIPT $SYS_NAME $BENCH_OUTPUT_FILE "$EXTRA_MNT_CMDS; /usr/bin/bash /test/corten_benchparsec.sh $BENCH $THREAD_COUNT $DO_ASTER_BREAKDOWN"
    done
done
