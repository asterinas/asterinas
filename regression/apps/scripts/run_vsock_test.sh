#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

VSOCK_DIR=/regression/vsock
cd ${VSOCK_DIR}

echo "Start vsock test......"
# ./vsock_server
./vsock_client
echo "Vsock test passed."
