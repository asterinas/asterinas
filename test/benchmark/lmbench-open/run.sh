#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running the LMbench open latency test ***"

touch test_file
/benchmark/bin/lmbench/lat_syscall -P 1 open test_file