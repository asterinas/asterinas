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

# Run apache bench
/usr/local/redis/bin/redis-benchmark -n 100000 -c 20 -t ping_mbulk

# The trap will automatically stop the guest VM when the script exits