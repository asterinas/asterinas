#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

test_fdatasync_other_filesystems() {
    ./fdatasync/fdatasync /
    rm -f /test_fdatasync.txt /test_fsync.txt
    ./fdatasync/fdatasync /exfat
    rm -f /exfat/test_fdatasync.txt /exfat/test_fsync.txt
}

test_mount_bind_file() {
    local file_a="/file_a.txt"
    local file_b="/file_b.txt"
    local content_a="initial content for file A"
    local content_b_new="new content written to file B"

    echo "$content_a" > "$file_a"
    touch "$file_b"

    mount --bind "$file_a" "$file_b"

    # Read from file_b and check if it matches file_a's content
    if [ "$(cat "$file_b")" != "$content_a" ]; then
        echo "Error: Read from bind-mounted file failed. Content mismatch."
        umount "$file_b"
        rm -f "$file_a" "$file_b"
        return 1
    fi

    echo "$content_b_new" > "$file_b"

    # Check if file_a's content is updated
    if [ "$(cat "$file_a")" != "$content_b_new" ]; then
        echo "Error: Write to bind-mounted file did not affect the source file."
        umount "$file_b"
        rm -f "$file_a" "$file_b"
        return 1
    fi

    umount "$file_b"
    rm -f "$file_a" "$file_b"
}

./ext/run_test.sh

echo "Start fdatasync test......"
test_fdatasync_other_filesystems
echo "All fdatasync test passed."

echo "Start mount bind file test......"
test_mount_bind_file
echo "All mount bind file test passed."

TEST_TMPDIR=/tmp TEST_DIRECT=0 ./empty_write/empty_write
TEST_TMPDIR=/exfat TEST_DIRECT=0 ./empty_write/empty_write
TEST_TMPDIR=/exfat TEST_DIRECT=1 ./empty_write/empty_write
TEST_TMPDIR=/ext2 TEST_DIRECT=0 ./empty_write/empty_write
TEST_TMPDIR=/ext2 TEST_DIRECT=1 ./empty_write/empty_write

./getcwd/getcwd

./inotify/inotify_align
./inotify/inotify_close
./inotify/inotify_o_path
./inotify/inotify_poll
./inotify/inotify_unlink

./isolation/chroot
./isolation/pivot_root

./mount/listmount
./mount/mount_api
./mount/mount_move
./mount/mount_propagation

./overlayfs/lchown_lower_symlink
./overlayfs/link_lower_file
./overlayfs/mknod_whiteout_rejected
./overlayfs/ovl_test
./overlayfs/readdir_small_buffer
./overlayfs/rmdir_lower_only_nonempty
./overlayfs/sparse_copyup_consistency
./overlayfs/xino_dino_identity

./procfs/dentry_cache
./procfs/fd
./procfs/getdents
./procfs/mountstats
./procfs/pid_mem
./procfs/proc_fd_open_fifo_after_setid
./procfs/proc_sys_kernel
./procfs/tid

./pseudofs/fallocate
./pseudofs/memfd_access_err
./pseudofs/memfd_create
./pseudofs/pseudo_dentry
./pseudofs/pseudo_dev_id
./pseudofs/pseudo_inode
./pseudofs/pseudo_mount

./rename/same_inode

./statx/btime

./symlink/symlink

./sync/sync

./tmpfile/tmpfile

./utimensat/utimensat
