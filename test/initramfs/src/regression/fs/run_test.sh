#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

check_file_size() {
    local file_name="$1"
    local expected_size="$2"

    if [ ! -f "$file_name" ]; then
        echo "Error: File does not exist."
        return 1
    fi

    actual_size=$(du -b "$file_name" | cut -f1)

    if [ "$actual_size" -eq "$expected_size" ]; then
        return 0
    else
        echo "Error: File size is incorrect: expected ${expected_size}, but got ${actual_size}."
        return 1
    fi
}

test_truncate_large() {
    local test_dir="$1"
    local test_file="$2"

    cd "$test_dir"

    # Test case for the big file feature
    for i in $(seq 1 10); do
        truncate -s 500M ${test_file}
        check_file_size ${test_file} $((500 * 1024 * 1024))
        truncate -s 2K ${test_file}
        check_file_size ${test_file} $((2 * 1024))
    done

    # Clean up
    rm -f ${test_file}
    sync
    cd -
}

test_fdatasync_other_filesystems() {
    ./fdatasync/fdatasync /
    rm -f /test_fdatasync.txt /test_fsync.txt
    ./fdatasync/fdatasync /exfat
    rm -f /exfat/test_fdatasync.txt /exfat/test_fsync.txt
}

run_test_list() {
    local list="$1"

    while read -r test_name; do
        case "$test_name" in ''|'#'*) continue ;; esac
        "./ext/$test_name"
    done < "$list"
}

run_ext_suite() {
    local suite_name="$1"
    local source="$2"
    local host_seed="$3"

    mkdir -p /ext-test
    mount --bind "$source" /ext-test

    echo "Start $suite_name shared EXT tests......"
    test_truncate_large /ext-test matrix_big_file.txt
    run_test_list ./ext/shared.tests
    ./fdatasync/fdatasync /ext-test
    rm -f /ext-test/test_fdatasync.txt /ext-test/test_fsync.txt
    if [ "$host_seed" = "yes" ]; then
        ./ext/host_seed
    fi
    echo "All $suite_name shared EXT tests passed."

    umount /ext-test
}

expect_mount_rejected() {
    local flavor="$1"
    local device="$2"
    local feature="$3"
    local mountpoint="/ext-negative/$feature-$flavor"

    mkdir -p "$mountpoint"
    if mount -t "$flavor" "$device" "$mountpoint" 2>/dev/null; then
        echo "$feature image unexpectedly mounted as $flavor" >&2
        umount "$mountpoint"
        return 1
    fi
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

run_ext_suite ext2 /ext2 no
mount --bind /ext2 /ext-test
run_test_list ./ext/ext2-only.tests
umount /ext-test
./ext/shared_block_device
echo "All ext2-only EXT tests passed."

mkdir -p /ext4
mount -t ext4 /dev/vdd /ext4
run_ext_suite ext4 /ext4 yes
mount --bind /ext4 /ext-test
run_test_list ./ext/ext4-only.tests
umount /ext-test
echo "All ext4-only EXT tests passed."
umount /ext4

expect_mount_rejected ext2 /dev/vdd extents
expect_mount_rejected ext4 /dev/vde journal
expect_mount_rejected ext4 /dev/vdf recover
expect_mount_rejected ext4 /dev/vdg 64bit
expect_mount_rejected ext4 /dev/vdh metadata_csum
expect_mount_rejected ext4 /dev/vdi unknown_incompat
echo "All unsupported EXT4 feature mounts were rejected."

echo "Start fdatasync test......"
test_fdatasync_other_filesystems
echo "All fdatasync test passed."

echo "Start mount bind file test......"
test_mount_bind_file
echo "All mount bind file test passed."

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

./overlayfs/ovl_test
./overlayfs/readdir_small_buffer

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
