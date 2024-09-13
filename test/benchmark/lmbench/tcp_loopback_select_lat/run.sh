#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running lmbench select TCP latency test ***"

/benchmark/bin/lmbench/lat_select -P 1 tcp