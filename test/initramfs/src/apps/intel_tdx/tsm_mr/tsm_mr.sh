#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

i=0
while [ $i -le 3 ]; do
    echo "Testing RTMR${i}..."
    dd if=/dev/urandom bs=48 count=1 > "/sys/devices/virtual/misc/tdx_guest/measurements/rtmr${i}:sha384"
    hd "/sys/devices/virtual/misc/tdx_guest/measurements/rtmr${i}:sha384"
    i=$((i + 1))
done

echo "All RTMR tests completed successfully"
