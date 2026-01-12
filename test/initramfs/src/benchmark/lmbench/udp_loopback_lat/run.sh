#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running lmbench UDP latency test ***"

/benchmark/bin/lmbench/lat_udp -s 127.0.0.1
/benchmark/bin/lmbench/lat_udp -P 1 127.0.0.1
/benchmark/bin/lmbench/lat_udp -S 127.0.0.1
