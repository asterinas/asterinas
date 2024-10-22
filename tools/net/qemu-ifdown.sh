#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

# Delete the TAP interface.
#
# It's used for the cleanup script of QEMU netdev, DO NOT run it manually.

if [ -n "$1" ]; then
    ip link set dev "$1" down
    ip link delete dev "$1"
    exit
else
    echo "Error: no interface specified"
    exit 1
fi