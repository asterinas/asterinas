#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running lmbench-fork ***"

/benchmark/bin/lmbench/lat_proc -P 1 fork