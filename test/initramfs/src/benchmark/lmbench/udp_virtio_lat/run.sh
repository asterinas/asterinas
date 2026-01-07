#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "Running lmbench UDP latency test over virtio-net..."

# Start the server
/benchmark/bin/lmbench/lat_udp -s 10.0.2.15

# Sleep for a long time to ensure VM won't exit
sleep 200