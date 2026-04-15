#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

./tcp_server &
sleep 0.2
./tcp_client

./udp_server &
sleep 0.2
./udp_client

./unix_server &
sleep 0.2
./unix_client

./listen_backlog
./privileged_ports
./send_buf_full
./sendmmsg
./socketpair
./sockoption
./sockoption_unix
./tcp_err
./tcp_poll
./tcp_reuseaddr
./udp_broadcast
./udp_err
./unix_datagram_err
./unix_seqpacket_err
./unix_stream_err

./netlink_route
./rtnl_err
./uevent_err
