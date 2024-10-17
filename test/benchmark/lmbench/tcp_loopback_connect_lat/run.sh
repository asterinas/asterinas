#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running lmbench TCP connection latency test***"

/benchmark/bin/lmbench/lat_connect -s 127.0.0.1
/benchmark/bin/lmbench/lat_connect 127.0.0.1
/benchmark/bin/lmbench/lat_connect -S 127.0.0.1
