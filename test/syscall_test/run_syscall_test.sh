#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

LTP_DIR=/opt/ltp
GVISOR_DIR=/opt/gvisor

echo "Running LTP syscall tests..."
if ! "$LTP_DIR/run_syscall_test.sh"; then
    echo "Error: LTP syscall tests failed." >&2
    exit 1
fi

echo "Running gVisor syscall tests..."
if ! "$GVISOR_DIR/run_syscall_test.sh"; then
    echo "Error: gVisor syscall tests failed." >&2
    exit 2
fi

echo "All syscall tests passed."
exit 0
