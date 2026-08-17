# SPDX-License-Identifier: MPL-2.0

mkdir -p "$TEST_DIR" "$SCRATCH_MNT"

for entry in "$TEST_DEV:$TEST_DIR:test" "$SCRATCH_DEV:$SCRATCH_MNT:scratch"; do
    tag="${entry%%:*}"; rest="${entry#*:}"; mnt="${rest%%:*}"; role="${rest##*:}"
    if ! mount -t "$FSTYP" "$tag" "$mnt"; then
        echo "Failed to mount virtio-fs tag $tag on $mnt ($role)" >&2
        exit 1
    fi
    if ! mountpoint -q "$mnt"; then
        echo "$mnt is not a mountpoint after mount(8) succeeded ($role)" >&2
        exit 1
    fi
done
