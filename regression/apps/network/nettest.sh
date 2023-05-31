#!/bin/sh

NETTEST_DIR=/network
cd ${NETTEST_DIR}
echo "Start net test......"
./tcp/server &
./tcp/client
./udp/server &
./udp/client

echo "All net test passed"
