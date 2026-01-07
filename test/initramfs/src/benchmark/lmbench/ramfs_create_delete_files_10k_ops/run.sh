#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running the LMbench file system create/delete test (Ramfs) ***"

/benchmark/bin/lmbench/lat_fs -s 10k -P 1 -W 30 -N 300