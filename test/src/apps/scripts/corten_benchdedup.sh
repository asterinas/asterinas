#!/bin/sh

# usage: corten_benchdedup.sh [tc|pt] thread_count [aster_breakdown]

MALLOC=$1
THREAD_COUNT=$2

if [ -z "$THREAD_COUNT" ] || [ "$MALLOC" != "tc" ] && [ "$MALLOC" != "pt" ]; then
    echo "Usage: $0 <tc|pt> <thread_count> [aster_breakdown]"
    exit 1
fi

DO_ASTER_BREAKDOWN=$3

# Copy the text file to ramfs
cp /benchmark/bin/vm_scale_bench_data/800MB.txt /root

if [ "$MALLOC" == "tc" ]; then
    BIN=/benchmark/bin/dedup/dedup-tc
    echo "Using tcmalloc"
else
    BIN=/benchmark/bin/dedup/dedup
    echo "Using ptmalloc"
fi

echo "***TEST_START***"

if [ "$DO_ASTER_BREAKDOWN" == "aster_breakdown" ]; then
    cat /proc/breakdown-counters
fi

$BIN -c -p -v -t $THREAD_COUNT -i /root/800MB.txt -o /test/output.dat.ddp

if [ "$DO_ASTER_BREAKDOWN" == "aster_breakdown" ]; then
    cat /proc/breakdown-counters
fi

echo "***TEST_END***"
