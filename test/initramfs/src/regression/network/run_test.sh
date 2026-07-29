#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

server_pid=
ready_file=

cleanup_server() {
    if [ -n "$server_pid" ]; then
        kill "$server_pid" 2>/dev/null || true
        wait "$server_pid" 2>/dev/null || true
        server_pid=
    fi

    rm -f "$ready_file" /tmp/test.sock
}

wait_for_server() {
    attempts=0
    while [ ! -e "$ready_file" ]; do
        if [ "$attempts" -eq 100 ]; then
            echo "Timed out waiting for $1 to become ready." >&2
            return 1
        fi

        if ! kill -0 "$server_pid" 2>/dev/null; then
            echo "$1 exited before becoming ready." >&2
            wait "$server_pid"
            return 1
        fi

        sleep 0.1
        attempts=$((attempts + 1))
    done
}

run_server_client_pair() {
    server=$1
    client=$2
    ready_file="/tmp/$(basename "$server")-ready-$$"

    rm -f "$ready_file"
    "$server" "$ready_file" &
    server_pid=$!

    trap cleanup_server EXIT HUP INT TERM
    wait_for_server "$server"
    "$client"
    wait "$server_pid"
    server_pid=
    trap - EXIT HUP INT TERM
    rm -f "$ready_file"
}

rm -f /tmp/test.sock
run_server_client_pair ./tcp_server ./tcp_client
run_server_client_pair ./udp_server ./udp_client
run_server_client_pair ./unix_server ./unix_client

./listen_backlog
./msg_peek
./msg_trunc
./privileged_ports
./send_buf_full
./sendmmsg
./socketpair
./sockoption
./sockoption_unix
./tcp_err
./tcp_poll
./tcp_reuseaddr
./tcp_wrapped_buffer_io
./udp_broadcast
./udp_err
./unix_datagram_err
./unix_seqpacket_err
./unix_stream_err

./netlink_route
./rtnl_err
./uevent_err
