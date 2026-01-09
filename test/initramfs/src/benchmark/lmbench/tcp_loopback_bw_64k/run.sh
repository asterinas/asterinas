#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running lmbench TCP bandwidth test ***"

/benchmark/bin/lmbench/bw_tcp -s 127.0.0.1 -b 1
/benchmark/bin/lmbench/bw_tcp -m 65536 -P 1 127.0.0.1
/benchmark/bin/lmbench/bw_tcp -S 127.0.0.1
