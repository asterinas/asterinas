#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

# TODO: This script simulates the process of mounting filesystems as performed by 
# a generic init process. It should later be replaced by the actual init process.
mount -t sysfs none /sys
mount -t proc none /proc
mount -t cgroup2 none /sys/fs/cgroup