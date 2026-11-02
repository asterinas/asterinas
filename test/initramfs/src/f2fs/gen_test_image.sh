#!/bin/bash
# SPDX-License-Identifier: MPL-2.0
#
# Regenerates the F2FS regression image (`f2fs_test.img.gz`).
#
# This script must run on a privileged Linux host whose kernel supports F2FS
# (the `f2fs` module must be loadable) and that has `mkfs.f2fs` installed.
# It creates a clean 128 MiB volume, populates it with deterministic content,
# and replaces the compressed image used by the regression tests.
#
# The file contents below are asserted by
# `test/initramfs/src/regression/fs/f2fs/f2fs_test.c`; keep both in sync.

set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
WORK_DIR=$(mktemp -d)
IMAGE="$WORK_DIR/f2fs.img"
MNT="$WORK_DIR/mnt"

cleanup() {
    mountpoint -q "$MNT" && umount "$MNT"
    rm -rf "$WORK_DIR"
}
trap cleanup EXIT

mkdir -p "$MNT"
truncate -s 128M "$IMAGE"
mkfs.f2fs -f -l ASTERF2 "$IMAGE"
mount -o loop "$IMAGE" "$MNT"

printf 'hello f2fs\n' > "$MNT/hello.txt"
: > "$MNT/empty.txt"

# 4 MiB of 0xA5: crosses the inode's address array into the first direct node.
dd if=/dev/zero bs=1M count=4 status=none | tr '\0' '\245' > "$MNT/big.bin"

# 9 MiB and 13 MiB periodic patterns: the latter exercises an indirect node.
python3 -c 'open("'"$MNT"'/huge.bin", "wb").write(bytes((i * 7 + 3) % 251 for i in range(9 * 1024 * 1024)))'
python3 -c 'open("'"$MNT"'/ind.bin", "wb").write(bytes((i * 13 + 5) % 253 for i in range(13 * 1024 * 1024)))'

# 1 MiB sparse file with a small data island in the middle.
truncate -s 1M "$MNT/sparse.bin"
printf 'hole-end\n' | dd of="$MNT/sparse.bin" bs=1 seek=524288 conv=notrunc status=none

mkdir -p "$MNT/dir1/sub"
printf 'nested\n' > "$MNT/dir1/nested.txt"
printf 'deep\n' > "$MNT/dir1/sub/deep.txt"

# Short inline symlink, long inline symlink, and a block-backed symlink.
ln -s hello.txt "$MNT/link"
ln -s "$(python3 -c 'print("a" * 3000)')" "$MNT/longlink"
ln -s "$(python3 -c 'print("b" * 3600)')" "$MNT/verylonglink"

# Enough entries to convert the directory from an inline dentry to blocks.
for i in $(seq 1 200); do
    printf 'file %s\n' "$i" > "$MNT/dir1/many_$i.txt"
done

mknod "$MNT/null_dev" c 1 3

sync
umount "$MNT"
gzip -9 -c "$IMAGE" > "$SCRIPT_DIR/f2fs_test.img.gz"
echo "Regenerated $SCRIPT_DIR/f2fs_test.img.gz"
