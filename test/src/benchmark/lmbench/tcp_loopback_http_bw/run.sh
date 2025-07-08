#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running lmbench HTTP latency test ***"

dd if=/dev/zero of=test_file bs=1M count=64
echo "test_file" > file_list
/benchmark/bin/lmbench/lmhttp &
sleep 1
/benchmark/bin/lmbench/lat_http 127.0.0.1 < file_list
/benchmark/bin/lmbench/lat_http -S 127.0.0.1
