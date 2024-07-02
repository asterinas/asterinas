#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running lmbench-syscall-read ***"

/benchmark/bin/lmbench/lat_syscall -P 1 read