#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -eu

FIO_ROOT="/benchmark/fio"
FIO_FS_TYPE=${FIO_FS_TYPE:-ext2}
FIO_WORKLOAD=${FIO_WORKLOAD:?FIO_WORKLOAD is required}
FIO_FS_DIR="${FIO_ROOT}/fs/${FIO_FS_TYPE}"
FIO_CONFIG="${FIO_FS_DIR}/config.sh"
FIO_PREPARE="${FIO_FS_DIR}/prepare.sh"

if [ ! -d "$FIO_FS_DIR" ]; then
    echo "Unsupported fio filesystem type: $FIO_FS_TYPE" >&2
    exit 2
fi
if [ ! -f "$FIO_CONFIG" ]; then
    echo "Missing fio filesystem config: $FIO_CONFIG" >&2
    exit 2
fi
if [ ! -f "$FIO_PREPARE" ]; then
    echo "Missing fio filesystem preparation script: $FIO_PREPARE" >&2
    exit 2
fi

# shellcheck source=/dev/null
. "$FIO_CONFIG"

# shellcheck source=/dev/null
. "$FIO_PREPARE"

echo "*** Running the FIO sequential ${FIO_WORKLOAD} test (${FIO_FS_TYPE}) ***"

case "$FIO_WORKLOAD" in
    read)
        FIO_NAME=seqread
        ;;
    write)
        FIO_NAME=seqwrite
        ;;
    *)
        echo "Unsupported fio workload: $FIO_WORKLOAD" >&2
        exit 2
        ;;
esac

/benchmark/bin/fio "-rw=${FIO_WORKLOAD}" "-filename=${FIO_TEST_FILE}" "-name=${FIO_NAME}" \
-size=1G -bs=1M \
-ioengine=sync -direct=1 -numjobs=1 -fsync_on_close=1 \
-time_based=1 -ramp_time=60 -runtime=100
