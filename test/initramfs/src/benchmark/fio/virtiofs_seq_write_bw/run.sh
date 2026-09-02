#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running the FIO buffered sequential write test (virtio-fs) ***"

mkdir -p /virtiofs
mountpoint -q /virtiofs || mount -t virtiofs aster-virtiofs /virtiofs
/benchmark/bin/fio -rw=write -filename=/virtiofs/fio-test -name=seqwrite \
    -size=1G -bs=1M \
    -ioengine=sync -direct=0 -numjobs=1 -fsync_on_close=1 \
    -time_based=1 -ramp_time=60 -runtime=100
