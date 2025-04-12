#!/bin/sh

# usage: corten_benchmetis.sh thread_count [aster_breakdown]

THREAD_COUNT=$1

if [ -z "$THREAD_COUNT" ]; then
    echo "Usage: $0 <thread_count>"
    exit 1
fi

DO_ASTER_BREAKDOWN=$2

echo "***TEST_START***"

if [ "$DO_ASTER_BREAKDOWN" == "aster_breakdown" ]; then
    cat /proc/breakdown-counters
fi

/benchmark/bin/metis/wrmem -s 1600 -p $THREAD_COUNT

if [ "$DO_ASTER_BREAKDOWN" == "aster_breakdown" ]; then
    cat /proc/breakdown-counters
fi

echo "***TEST_END***"
