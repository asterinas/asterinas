# SPDX-License-Identifier: MPL-2.0

mkdir -p "$TEST_DIR" "$SCRATCH_MNT"

# Mount xfstests images with explicit error checking so a mount failure is not
# silently skipped (which would cause ./check to run against empty directories
# and still print the "all passed" success line).
for entry in "$TEST_DEV:$TEST_DIR:test" "$SCRATCH_DEV:$SCRATCH_MNT:scratch"; do
    dev="${entry%%:*}"; rest="${entry#*:}"; mnt="${rest%%:*}"; role="${rest##*:}"
    if [ ! -b "$dev" ]; then
        echo "Expected $dev to be a block device for xfstests $role" >&2
        exit 1
    fi
    if ! mount -t "$FSTYP" "$dev" "$mnt"; then
        echo "Failed to mount $dev on $mnt ($role)" >&2
        exit 1
    fi
    if ! mountpoint -q "$mnt"; then
        echo "$mnt is not a mountpoint after mount(8) succeeded ($role)" >&2
        exit 1
    fi
done
