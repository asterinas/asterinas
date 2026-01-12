#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running the LMbench stat latency test ***"

touch testfile
/benchmark/bin/lmbench/lat_syscall -P 1 -W 1000 -N 1000 stat testfile