#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

NETTEST_DIR=/regression/network
cd ${NETTEST_DIR}

echo "Start network test......"

./tcp_server &
./tcp_client
./udp_server &
./udp_client
./unix_server &
./unix_client
./socketpair
./sockoption
./listen_backlog
# ./send_buf_full
./http_server &
./http_client
./tcp_err
./udp_err

echo "All network test passed"
