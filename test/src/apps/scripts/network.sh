#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

NETTEST_DIR=/test/network
cd ${NETTEST_DIR}

echo "Start network test......"

./tcp_server &
sleep 0.2
./tcp_client

./udp_server &
sleep 0.2
./udp_client

./unix_server &
sleep 0.2
./unix_client

./http_server &
sleep 0.2
./http_client

./socketpair
./sockoption
./sockoption_unix
./listen_backlog
./send_buf_full
./tcp_err
./tcp_poll
./tcp_reuseaddr
./udp_err
./unix_stream_err
./unix_seqpacket_err

./netlink_route
./rtnl_err
./uevent_err

echo "All network test passed"
