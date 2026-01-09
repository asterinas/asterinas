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

# Run apache bench
echo "Running apache bench connected to $GUEST_SERVER_IP_ADDRESS"
redis-benchmark -h $GUEST_SERVER_IP_ADDRESS -n 100000 -c 20 -t get

# The trap will automatically stop the guest VM when the script exits