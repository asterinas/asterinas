#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "Running Memcached server"
/benchmark/bin/memcached --user=root --listen=10.0.2.15
