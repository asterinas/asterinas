#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running the LMbench file system create/delete test (Ext2) ***"

/benchmark/bin/lmbench/lat_fs -s 0k -P 1 /ext2