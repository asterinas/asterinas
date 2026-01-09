#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running the LMbench mmap bandwidth test ***"

dd if=/dev/zero of=/ext2/test_file bs=1M count=256
/benchmark/bin/lmbench/bw_mmap_rd -W 30 -N 300 256m mmap_only /ext2/test_file