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
/usr/local/benchmark/iperf/bin/iperf3 -c 127.0.0.1 -f m

# The trap will automatically stop the guest VM when the script exits