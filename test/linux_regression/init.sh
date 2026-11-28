#!/usr/bin/env sh

# SPDX-License-Identifier: MPL-2.0

set -e
export PATH=/bin
export HOME=/
trap 'poweroff -f' EXIT

# Mount the guest filesystems and enable loopback for the suite.
mount -t proc proc /proc
mount -t sysfs sysfs /sys
mkdir -p /dev/pts /dev/shm
mount -t devpts devpts /dev/pts
mount -t tmpfs tmpfs /dev/shm
mount -t tmpfs tmpfs /tmp
mount -t configfs configfs /sys/kernel/config
mount -t cgroup2 cgroup2 /sys/fs/cgroup
mount -t ext2 /dev/vda /ext2
ip link set lo up

# Run the suite in a new session on `ttyS1` (controlling terminal for `process/job_control`).
setsid -c /test_session.sh </dev/ttyS1 >/dev/ttyS1 2>&1 || true
