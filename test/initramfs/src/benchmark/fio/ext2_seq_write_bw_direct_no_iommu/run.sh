#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running the FIO sequential write test (Ext2) ***"

sh /benchmark/fio/common/run_fio.sh -rw=write -filename=/ext2/fio-test -name=seqwrite \
    -size=1G -bs=1M \
    -ioengine=sync -direct=1 -numjobs=1 -fsync_on_close=1 \
    -time_based=1 -ramp_time=60 -runtime=100
