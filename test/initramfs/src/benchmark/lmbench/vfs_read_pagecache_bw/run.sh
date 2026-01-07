#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running the LMbench file read bandwidth test ***"

dd if=/dev/zero of=/ext2/test_file bs=1M count=512
/benchmark/bin/lmbench/bw_file_rd -P 1 -W 30 -N 300 512m io_only /ext2/test_file
