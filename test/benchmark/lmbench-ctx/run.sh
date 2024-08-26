#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running the LMbench context switch latency test ***"

/benchmark/bin/lat_ctx -P 1 18