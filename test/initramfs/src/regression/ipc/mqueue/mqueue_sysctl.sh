#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

# Reads the value of a `/proc/sys/fs/mqueue` sysctl file.
read_sysctl() {
    cat "$1"
}

# Asserts that two strings are equal.
assert_eq() {
    if [ "$1" != "$2" ]; then
        echo "FAIL: expected '$2', got '$1'" >&2
        exit 1
    fi
}

# Default values.
assert_eq "$(read_sysctl /proc/sys/fs/mqueue/queues_max)" "256"
assert_eq "$(read_sysctl /proc/sys/fs/mqueue/msg_max)" "10"
assert_eq "$(read_sysctl /proc/sys/fs/mqueue/msg_default)" "10"
assert_eq "$(read_sysctl /proc/sys/fs/mqueue/msgsize_max)" "8192"
assert_eq "$(read_sysctl /proc/sys/fs/mqueue/msgsize_default)" "8192"

# Writing updates the value.
printf '20\n' > /proc/sys/fs/mqueue/msg_max
assert_eq "$(read_sysctl /proc/sys/fs/mqueue/msg_max)" "20"
printf '10\n' > /proc/sys/fs/mqueue/msg_max
assert_eq "$(read_sysctl /proc/sys/fs/mqueue/msg_max)" "10"

# Negative values are rejected.
if printf '%s\n' -1 > /proc/sys/fs/mqueue/msg_max 2>/dev/null; then
    echo "FAIL: negative value was accepted" >&2
    exit 1
fi

echo "mqueue sysctl tests passed."
