#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

./drm/device_node
./drm/get_info
./drm/master

./pty/close_pty
./pty/open_ptmx
./pty/open_pty
./pty/open_pty_peer
./pty/pty_blocking
./pty/pty_packet_mode
./pty/signal_char
./pty/termios2

./vt/vt_ioctl

./devtmpfs_mode
./evdev
./framebuffer
./full
./hwrng
./nvme
./random
