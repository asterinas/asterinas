#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

# To successfully run the vsock test, you should
# 1. Run vsock server binding port 1234 on the host, before running ./vsock_client
# 2. Run vsock client connecting (cid,port)=(3,4321) on the host, after running ./vsock_server

set -e

VSOCK_DIR=/test/vsock
cd ${VSOCK_DIR}

echo "Start vsock test......"
./vsock_client
./vsock_server
echo "Vsock test passed."
