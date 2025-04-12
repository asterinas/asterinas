#!/bin/sh

# usage: corten_benchdedup.sh thread_count [aster_breakdown]

THREAD_COUNT=$1

if [ -z "$THREAD_COUNT" ]; then
    echo "Usage: $0 <thread_count> [aster_breakdown]"
    exit 1
fi

DO_ASTER_BREAKDOWN=$2

export LD_LIBRARY_PATH=/usr/lib/jvm/java-21-openjdk-amd64/bin/../lib/

if [ "$DO_ASTER_BREAKDOWN" == "aster_breakdown" ]; then
    cat /proc/breakdown-counters
fi

/usr/bin/java -Xmx16384m -Xms1m -cp /test/scale/ jvm_thread $THREAD_COUNT
/usr/bin/java -Xmx16384m -Xms1m -cp /test/scale/ jvm_thread $THREAD_COUNT
/usr/bin/java -Xmx16384m -Xms1m -cp /test/scale/ jvm_thread $THREAD_COUNT
/usr/bin/java -Xmx16384m -Xms1m -cp /test/scale/ jvm_thread $THREAD_COUNT
/usr/bin/java -Xmx16384m -Xms1m -cp /test/scale/ jvm_thread $THREAD_COUNT

if [ "$DO_ASTER_BREAKDOWN" == "aster_breakdown" ]; then
    cat /proc/breakdown-counters
fi

unset LD_LIBRARY_PATH