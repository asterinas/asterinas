#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -eu

XFSTESTS_DIR=${XFSTESTS_DIR:-/opt/xfstests}
TEST_DEV=${XFSTESTS_TEST_DEV:-/dev/vdc}
SCRATCH_DEV=${XFSTESTS_SCRATCH_DEV:-/dev/vdd}
export TEST_DEV SCRATCH_DEV

# Mount xfstests images with explicit error checking so a mount failure is not
# silently skipped (which would cause ./check to run against empty directories
# and still print the "all passed" success line).
for entry in "$TEST_DEV:$XFSTESTS_DIR/test:test" "$SCRATCH_DEV:$XFSTESTS_DIR/scratch:scratch"; do
    dev="${entry%%:*}"; rest="${entry#*:}"; mnt="${rest%%:*}"; role="${rest##*:}"
    if [ ! -b "$dev" ]; then
        echo "Expected $dev to be a block device for xfstests $role" >&2
        exit 1
    fi
    if ! mount -t ext2 "$dev" "$mnt"; then
        echo "Failed to mount $dev on $mnt ($role)" >&2
        exit 1
    fi
    if ! mountpoint -q "$mnt"; then
        echo "$mnt is not a mountpoint after mount(8) succeeded ($role)" >&2
        exit 1
    fi
done

./check "$@"
