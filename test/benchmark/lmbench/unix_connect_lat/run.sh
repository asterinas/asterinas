#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running the LMbench unix connect latency test ***"

/benchmark/bin/lat_unix_connect -s
/benchmark/bin/lat_unix_connect -P 1