#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

# Create a TAP interface.
#
# It's used for the startup script of QEMU netdev, DO NOT run it manually.

# This IP address should be set the same as gateway address of Asterinas
IP=10.0.2.2/24

if [ -n "$1" ]; then
    ip addr add $IP dev "$1"
    ip link set dev "$1" up
    exit
else
    echo "Error: no interface specified"
    exit 1
fi