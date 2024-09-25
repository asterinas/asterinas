#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running lmbench-fcntl ***"

/benchmark/bin/lmbench/lat_fcntl -P 1 -W 30 -N 200