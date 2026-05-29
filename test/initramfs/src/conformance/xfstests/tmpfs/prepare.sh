# SPDX-License-Identifier: MPL-2.0

# Patch xfstests' tmpfs defaults here so the workaround stays scoped to tmpfs.
sed -i 's/\[ -z "TEST_DEV" \]/[ -z "$TEST_DEV" ]/' "$XFSTESTS_DIR/common/config"
sed -i 's/\[ -z "SCRATCH_DEV" \]/[ -z "$SCRATCH_DEV" ]/' "$XFSTESTS_DIR/common/config"
sed -i 's/export TEST_DEV=tmpfs_scratch/export SCRATCH_DEV=tmpfs_scratch/' "$XFSTESTS_DIR/common/config"

mkdir -p "$TEST_DIR" "$SCRATCH_MNT"

for entry in "$TEST_DEV:$TEST_DIR:test" "$SCRATCH_DEV:$SCRATCH_MNT:scratch"; do
    dev="${entry%%:*}"; rest="${entry#*:}"; mnt="${rest%%:*}"; role="${rest##*:}"
    if ! mount -t "$FSTYP" "$dev" "$mnt"; then
        echo "Failed to mount $dev on $mnt ($role)" >&2
        exit 1
    fi
    if ! mountpoint -q "$mnt"; then
        echo "$mnt is not a mountpoint after mount(8) succeeded ($role)" >&2
        exit 1
    fi
done
