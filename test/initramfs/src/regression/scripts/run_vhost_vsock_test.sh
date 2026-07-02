#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

cd /test/network
./vhost_vsock
echo "Vhost-vsock test passed."
