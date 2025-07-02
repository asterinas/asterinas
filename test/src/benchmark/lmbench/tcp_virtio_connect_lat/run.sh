#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "Running lmbench TCP connect latency test over virtio-net..."

# Start the server
benchmark/bin/lmbench/lat_connect -s 10.0.2.15 -b 1000

# Sleep for a long time to ensure VM won't exit
sleep 200