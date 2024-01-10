#!/bin/sh

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
./listen_backlog
./send_buf_full
./tcp_err
./udp_err

echo "All network test passed"
