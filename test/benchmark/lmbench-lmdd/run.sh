#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running the LMbench lmdd test ***"

echo -n "lmdd result: " & /benchmark/bin/lmbench/lmdd if=/dev/zero of=/ext2/test_file bs=1M count=512