#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "Running lmbench TCP latency over virtio-net..."

# Start the server
/benchmark/bin/lat_tcp -s 10.0.2.15 -b 1

# Sleep for a long time to ensure VM won't exit
sleep 200
