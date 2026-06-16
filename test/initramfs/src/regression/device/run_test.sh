#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

./drm/device_node
./drm/get_info
./drm/access_control

./pty/close_pty
./pty/open_ptmx
./pty/open_pty
./pty/pty_blocking
./pty/pty_packet_mode

./vt/vt_ioctl

./devtmpfs_mode
./evdev
./framebuffer
./full
./hwrng
./nvme
./random
