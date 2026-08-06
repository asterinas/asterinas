#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

echo "*** Running boot_lat_100mb benchmark ***"
UPTIME=$(awk '{print $1}' /proc/uptime)
echo "Boot time: ${UPTIME} seconds"
