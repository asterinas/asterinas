#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

cd "$(dirname "$0")"

echo "[capability] running capabilities"
./capabilities

echo "[capability] running capset"
./capset

echo "[capability] running setgroups"
./setgroups

echo "[capability] running trusted_xattr"
./trusted_xattr

echo "[capability] running kill"
./kill

echo "[capability] running reboot"
./reboot

echo "[capability] running execve"
./execve
