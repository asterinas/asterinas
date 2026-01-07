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

# Run lmbench tcp client
echo "Running lmbench tcp client connected to $GUEST_SERVER_IP_ADDRESS"
# The -W parameter of bw_tcp refers to the number of bytes transferred during the warm-up phase.
bw_tcp -m 65536 -P 1 -W 50000000 -N 100 $GUEST_SERVER_IP_ADDRESS

# The trap will automatically stop the guest VM when the script exits
