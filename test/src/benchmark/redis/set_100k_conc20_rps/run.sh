#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "Running redis server"
/usr/local/redis/bin/redis-server /etc/redis.conf
