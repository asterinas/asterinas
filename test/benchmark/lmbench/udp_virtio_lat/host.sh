#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

# Function to stop the guest VM
stop_guest() {
    echo "Stopping guest VM..."
    # `-r` means if there's no qemu, the kill won't be executed
    pgrep qemu | xargs -r kill
}

# Trap EXIT signal to ensure guest VM is stopped on script exit
trap stop_guest EXIT

# Run lmbench udp client
echo "Running lmbench udp client connected to $GUEST_SERVER_IP_ADDRESS"
# Warm up: We intentionally run another test for warmup here. 
# Note that we can't use -W option for warmup here because it will fail due to receiving timeout.
/usr/local/benchmark/lmbench/lat_udp -P 1 -N 10 $GUEST_SERVER_IP_ADDRESS 2>&1 > /dev/null
/usr/local/benchmark/lmbench/lat_udp -P 1 -N 10 $GUEST_SERVER_IP_ADDRESS

# The trap will automatically stop the guest VM when the script exits