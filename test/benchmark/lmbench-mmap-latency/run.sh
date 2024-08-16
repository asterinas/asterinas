#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running the LMbench mmap latency test ***"

dd if=/dev/zero of=/ext2/test_file bs=1M count=256
/benchmark/bin/lmbench/lat_mmap 256m /ext2/test_file