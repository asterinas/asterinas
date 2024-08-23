#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

echo "*** Running the iperf3 localhost TCP bitrate test ***"

/benchmark/bin/iperf3 -B 127.0.0.1 -s -f m -D
/benchmark/bin/iperf3 -c 127.0.0.1 -f m