#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running the FIO buffered sequential read test (Ext2) ***"

sh /benchmark/fio/common/run_fio.sh -rw=read -filename=/ext2/fio-test -name=seqread \
    -size=1G -bs=1M \
    -ioengine=sync -direct=0 -numjobs=1 -fsync_on_close=1 \
    -time_based=1 -ramp_time=60 -runtime=100
