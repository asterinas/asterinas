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

FILESIZE=4096

# Run apache bench
echo "Warm up......"
ab -n 20000 -c 1 http://${GUEST_SERVER_IP_ADDRESS}:8080/${FILESIZE}bytes.html >/dev/null 2>&1
echo "Running apache bench connected to $GUEST_SERVER_IP_ADDRESS"
ab -n 10000 -c 1 http://${GUEST_SERVER_IP_ADDRESS}:8080/${FILESIZE}bytes.html

# The trap will automatically stop the guest VM when the script exits