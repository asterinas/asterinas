#!/bin/sh

NETTEST_DIR=/regression/network
cd ${NETTEST_DIR}
echo "Start net test......"

./tcp_server 0.0.0.0 &
./tcp_client 127.0.0.1

./tcp_server 127.0.0.1 &
./tcp_client 0.0.0.0

./tcp_server 0.0.0.0 &
./tcp_client 0.0.0.0

./tcp_server 127.0.0.1 &
./tcp_client 127.0.0.1

./udp_server &
./udp_client

./unix_server &
./unix_client

./socketpair

echo "All net test passed"
