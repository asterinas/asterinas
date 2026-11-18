#!/bin/sh
# SPDX-License-Identifier: MPL-2.0
#
# Generate one clean ext2 base image and two malformed samples from copies.
# Run from the repository root. Output goes to test/initramfs/build/fault_images/.
# Only this script and the test sources are committed; .img files are build
# artifacts and must not be added to git.
set -eu

OUT=${1:-test/initramfs/build/fault_images}
mkdir -p "$OUT"

truncate -s 64M "$OUT/base.img"
mke2fs -q -t ext2 -b 4096 -F -I 256 "$OUT/base.img"

# #3866: group descriptor 0 starts at block 1. Point bg_inode_table at block
# 2 (the block bitmap) so inode-table reads see a zero i_links_count for the
# root inode. Four bytes change.
cp "$OUT/base.img" "$OUT/bad_root.img"
printf '\002\000\000\000' | dd of="$OUT/bad_root.img" bs=1 seek=4104 conv=notrunc status=none

# #4128: create a file large enough to need an indirect block, then replace
# that indirect block with out-of-range pointers (0xFFFFFFFF).
cp "$OUT/base.img" "$OUT/bad_indirect.img"
MNT=$(mktemp -d)
mount -o loop "$OUT/bad_indirect.img" "$MNT"
dd if=/dev/zero of="$MNT/ind2" bs=4096 count=13 status=none
umount "$MNT"
rmdir "$MNT"

IND=$(debugfs -R "stat /ind2" "$OUT/bad_indirect.img" 2>/dev/null \
      | tr ',' '\n' | grep -o '(IND):[0-9]*' | cut -d: -f2)
if [ -z "$IND" ]; then
    echo "make_fault_images: could not find the indirect block number" >&2
    exit 1
fi
python3 - "$OUT/bad_indirect.img" "$IND" <<'PY'
import sys
path, ind = sys.argv[1], int(sys.argv[2])
with open(path, 'r+b') as f:
    f.seek(ind * 4096)
    f.write(b'\xff' * 4096)
PY

echo "generated $OUT/base.img, bad_root.img, bad_indirect.img"
