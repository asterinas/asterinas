#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

cd "$(dirname "$0")"

./mem
./pci
./virtio
./block

echo "All device model tests passed."
