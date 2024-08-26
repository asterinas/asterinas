#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running the LMbench fstat latency test ***"

touch test_file
/benchmark/bin/lat_syscall -P 1 fstat test_file