#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

DIR=$(dirname "$0")
LOADER="$DIR/loader"
PACKET_TEST="$DIR/packet_test"

run_case() {
	prog="$1"
	expect="$2"
	payload="$3"

	"$LOADER" --prog "$DIR/$prog" &
	loader_pid=$!
	trap 'kill "$loader_pid" 2>/dev/null || true' INT TERM EXIT
	# Give the hook time to attach before sending packets.
	sleep 0.2
	"$PACKET_TEST" --expect "$expect" "$payload"
	kill "$loader_pid" 2>/dev/null || true
	wait "$loader_pid" 2>/dev/null || true
	trap - INT TERM EXIT
}

run_case accept.bin pass "accept payload"
run_case drop.bin drop "drop payload"
run_case check_meta.bin pass "meta payload"

echo "netfilter eBPF demo passed"
