#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running the LMbench lmdd test ***"

dd if=/dev/zero of=/ext2/zero_file bs=1M count=512
echo -n "lmdd result: " & /benchmark/bin/lmbench/lmdd if=/ext2/zero_file of=/ext2/test_file