# SPDX-License-Identifier: MPL-2.0

mkdir -p "$TEST_DIR" "$SCRATCH_MNT"

for dev in "$TEST_DEV" "$SCRATCH_DEV"; do
    if [ ! -b "$dev" ]; then
        echo "Expected $dev to be a block device for overlay xfstests" >&2
        exit 1
    fi
done

# Scoped Asterinas compatibility: move init_rc after the overrides so
# directory-backed overlay mounts pass init_rc checks.
if ! grep -q "Asterinas-specific compatibility overrides" \
        "$XFSTESTS_DIR/common/rc"; then
    sed '/^init_rc$/d' "$XFSTESTS_DIR/common/rc" \
        > "$XFSTESTS_DIR/common/rc.runtime"
    cat "$XFSTESTS_FS_DIR/common_rc_asterinas_compat.sh" \
        >> "$XFSTESTS_DIR/common/rc.runtime"
    printf '%s\n' 'init_rc' >> "$XFSTESTS_DIR/common/rc.runtime"
    mv "$XFSTESTS_DIR/common/rc.runtime" "$XFSTESTS_DIR/common/rc"
fi
