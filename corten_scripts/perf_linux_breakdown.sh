#!/bin/bash

set -ex

# usage: flamegraph.sh app

SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
FLAME_GRAPH_DIR=/root/FlameGraph

APP=$1

if [ -z "$APP" ]; then
    echo "Usage: $0 <app> <thread_count>"
    exit 1
fi

OUT_DIR="$SCRIPT_DIR/../corten_outputs/${APP}_Linux_breakdowns"
mkdir -p $OUT_DIR

INITRAMFS_DIR="$SCRIPT_DIR/../test/build/initramfs"

TEST_DB=$INITRAMFS_DIR/benchmark/test_db
psearchy_rm_tdb() {
  if [ -d "$TEST_DB" ]; then
    rm -rf $TEST_DB
  fi
}
psearchy_prepare_tdb() {
  psearchy_rm_tdb

  # Use shell arithmetic to increment i
  i=0
  while [ "$i" -lt "$THREAD_COUNT" ]; do
    dir_path="$TEST_DB/db$i"
    mkdir -p "$dir_path"
    i=$((i + 1))
  done

  echo "Created $THREAD_COUNT directories in $TEST_DB"
}

THREAD_COUNTS=(1 2 4 8 16 32 64 128 192 256 320 384)
for THREAD_COUNT in "${THREAD_COUNTS[@]}"; do

    case $APP in
        jvm_thread)
            NUM_ITERATIONS=100
            perf record -F 99 -a -g -- bash -c "for i in \$(seq 1 $NUM_ITERATIONS); do /usr/bin/java -Xmx16384m -Xms1m -cp $INITRAMFS_DIR/test/scale/ jvm_thread $THREAD_COUNT; done"
            ;;
        metis)
            perf record -F 99 -a -g -- $INITRAMFS_DIR/benchmark/bin/metis/wrmem -s 1600 -p $THREAD_COUNT
            ;;
        dedup)
            INPUT="/root/mm-scalability-benchmark/data/800MB.txt"
            perf record -F 99 -a -g -- $INITRAMFS_DIR/benchmark/bin/dedup/dedup -c -p -v -t $THREAD_COUNT -i $INPUT -o /tmp/output.dat.ddp
            ;;
        dedup-tc)
            INPUT="/root/mm-scalability-benchmark/data/800MB.txt"
            perf record -F 99 -a -g -- $INITRAMFS_DIR/benchmark/bin/dedup/dedup-tc -c -p -v -t $THREAD_COUNT -i $INPUT -o /tmp/output.dat.ddp
            ;;
        psearchy)
            psearchy_prepare_tdb
            perf record -F 99 -a -g -- $INITRAMFS_DIR/benchmark/bin/psearchy/pedsort -t $TEST_DB/db -c $THREAD_COUNT -m 512 < $INITRAMFS_DIR/benchmark/bin/psearchy/linux_files_index
            psearchy_rm_tdb
            ;;
        psearchy-tc)
            psearchy_prepare_tdb
            perf record -F 99 -a -g -- $INITRAMFS_DIR/benchmark/bin/psearchy/pedsort-tc -t $TEST_DB/db -c $THREAD_COUNT -m 512 < $INITRAMFS_DIR/benchmark/bin/psearchy/linux_files_index
            psearchy_rm_tdb
            ;;
        *)
            echo "Unknown app: $APP"
            exit 1
            ;;
    esac

    perf script | $FLAME_GRAPH_DIR/stackcollapse-perf.pl > out.perf-folded
    # $FLAME_GRAPH_DIR/flamegraph.pl out.perf-folded > out.svg

    OUT_FILE_NAME="${THREAD_COUNT}"

    mv out.perf-folded $OUT_DIR/$OUT_FILE_NAME.perf-folded

    rm perf.data

    echo "Flamegraph saved to $OUT_DIR/$OUT_FILE_NAME.perf-folded"
    echo "Flamegraph generation completed."
done
