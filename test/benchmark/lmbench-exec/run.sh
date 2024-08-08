#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running the LMbench exec latency test ***"

if [ ! -d /tmp ]; then
    mkdir /tmp
fi
cp /benchmark/bin/lmbench/hello /tmp/
/benchmark/bin/lmbench/lat_proc -P 1 exec