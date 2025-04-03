#!/bin/sh

# usage: corten_benchdedup.sh thread_count

THREAD_COUNT=$1

if [ -z "$THREAD_COUNT" ]; then
    echo "Usage: $0 <thread_count>"
    exit 1
fi

export LD_LIBRARY_PATH=/usr/lib/jvm/java-21-openjdk-amd64/bin/../lib/

/usr/bin/java -Xmx16384m -Xms1m -cp /test/scale/ jvm_thread $THREAD_COUNT
/usr/bin/java -Xmx16384m -Xms1m -cp /test/scale/ jvm_thread $THREAD_COUNT
/usr/bin/java -Xmx16384m -Xms1m -cp /test/scale/ jvm_thread $THREAD_COUNT
/usr/bin/java -Xmx16384m -Xms1m -cp /test/scale/ jvm_thread $THREAD_COUNT
/usr/bin/java -Xmx16384m -Xms1m -cp /test/scale/ jvm_thread $THREAD_COUNT

unset LD_LIBRARY_PATH