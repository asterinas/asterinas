#!/bin/sh

# usage: corten_benchmetis.sh thread_count

THREAD_COUNT=$1

if [ -z "$THREAD_COUNT" ]; then
    echo "Usage: $0 <thread_count>"
    exit 1
fi

echo "***TEST_START***"

/benchmark/bin/metis/wrmem -s 1600 -p $THREAD_COUNT

echo "***TEST_END***"
