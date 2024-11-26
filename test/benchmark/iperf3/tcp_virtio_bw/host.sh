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

# Run iperf3 client
echo "Running iperf3 client connected to $GUEST_SERVER_IP_ADDRESS"
/usr/local/benchmark/iperf/bin/iperf3 -c $GUEST_SERVER_IP_ADDRESS -f m

# The trap will automatically stop the guest VM when the script exits