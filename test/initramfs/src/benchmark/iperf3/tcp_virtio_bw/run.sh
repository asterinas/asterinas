#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "Running iperf3 server..."
/benchmark/bin/iperf3 -s -B 10.0.2.15 --one-off