#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running the LMbench semaphore latency test ***"

/benchmark/bin/lat_sem -P 1 -N 21