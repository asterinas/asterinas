#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running the LMbench shell latency test ***"

cp /benchmark/bin/lmbench/hello /tmp/
/benchmark/bin/lmbench/lat_proc -P 1 shell