#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

./pty/close_pty
./pty/open_ptmx
./pty/open_pty
./pty/pty_blocking
./pty/pty_packet_mode
./evdev
./framebuffer
./full
./random
