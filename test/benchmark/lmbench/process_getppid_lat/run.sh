#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running lmbench-getpid ***"

/benchmark/bin/lat_syscall -P 1 null