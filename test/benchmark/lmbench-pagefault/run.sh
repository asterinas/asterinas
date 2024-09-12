#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running the LMbench page fault latency test ***"

dd if=/dev/zero of=/ext2/test_file bs=1M count=4
/benchmark/bin/lmbench/lat_pagefault -P 1 /ext2/test_file