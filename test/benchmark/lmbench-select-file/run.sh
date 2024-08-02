#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running the LMbench select file latency test ***"

/benchmark/bin/lmbench/lat_select -P 1 file