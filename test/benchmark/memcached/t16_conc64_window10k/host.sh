#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

# Function to stop the guest VM
stop_guest() {
    echo "Stopping guest VM..."
    pgrep qemu | xargs kill
}

# Trap EXIT signal to ensure guest VM is stopped on script exit
trap stop_guest EXIT

# Run memaslap bench
echo "Running memaslap bench connected to $GUEST_SERVER_IP_ADDRESS"
/usr/local/benchmark/libmemcached/bin/memaslap -s $GUEST_SERVER_IP_ADDRESS:11211 -t 30s -T 16 -c 64 -w 10k -S 1s

# The trap will automatically stop the guest VM when the script exits