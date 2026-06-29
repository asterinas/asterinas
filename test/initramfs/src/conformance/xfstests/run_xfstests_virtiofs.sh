#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -eu

XFSTESTS_DIR=${XFSTESTS_DIR:-/opt/xfstests}
TEST_DEV=${XFSTESTS_TEST_DEV:-xfstest}
SCRATCH_DEV=${XFSTESTS_SCRATCH_DEV:-xfsscratch}
export TEST_DEV SCRATCH_DEV

for entry in "$TEST_DEV:$XFSTESTS_DIR/test:test" "$SCRATCH_DEV:$XFSTESTS_DIR/scratch:scratch"; do
    tag="${entry%%:*}"; rest="${entry#*:}"; mnt="${rest%%:*}"; role="${rest##*:}"
    if ! mount -t virtiofs "$tag" "$mnt"; then
        echo "Failed to mount virtiofs tag $tag on $mnt ($role)" >&2
        exit 1
    fi
    if ! mountpoint -q "$mnt"; then
        echo "$mnt is not a mountpoint after mount(8) succeeded ($role)" >&2
        exit 1
    fi
done

./check -virtiofs "$@"
