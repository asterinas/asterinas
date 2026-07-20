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
    local ext2_dir="$1"
    local test_file="$2"

    cd ${ext2_dir}

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

test_fdatasync() {
    ./fdatasync/fdatasync /
    rm -f /test_fdatasync.txt
    ./fdatasync/fdatasync /ext2
    rm -f /ext2/test_fdatasync.txt
    ./fdatasync/fdatasync /exfat
    rm -f /exfat/test_fdatasync.txt
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

# The ext feature matrix: the same shared suite (ext/ext.tests) runs against
# every image shape x mount-flavor cell the unified driver accepts,
# bind-mounted onto the common root the test binaries are built around.
# Serial-numbered matrix disks attach after the established ones, so
# vda..vdd keep their meaning. (/ext2 itself -- /dev/vda mounted by init with
# -t ext2 -- is exercised by the fdatasync test below, now served by the
# unified driver.)
EXT_TEST_ROOT=/ext-test

run_ext_suite() {
    local flavor="$1" device="$2" mnt="$3" seed="$4"

    mkdir -p "$mnt" "$EXT_TEST_ROOT"
    mount -t "$flavor" "$device" "$mnt"
    mount --bind "$mnt" "$EXT_TEST_ROOT"

    test_truncate_large "$EXT_TEST_ROOT" "matrix_big_file.txt"
    while read -r name; do
        case "$name" in ''|'#'*) continue ;; esac
        # host_seed reads a file mke2fs seeded on the host; only the seeded
        # image cell carries it.
        if [ "$name" = "host_seed" ] && [ "$seed" != "seed" ]; then
            continue
        fi
        "./ext/$name"
    done < ./ext/ext.tests

    umount "$EXT_TEST_ROOT"
    umount "$mnt"
}

expect_mount_rejected() {
    local flavor="$1" device="$2" mnt="$3"

    mkdir -p "$mnt"
    if mount -t "$flavor" "$device" "$mnt" 2>/dev/null; then
        echo "ext matrix: unexpectedly mounted $device as $flavor" >&2
        umount "$mnt"
        return 1
    fi
}

echo "Start ext matrix fs test......"
# Positive cells: every image under every type name that legally accepts it.
# ext2-format images (and the extent-free, journal-free ext4_noextents) mount
# under both names; extent or journaled volumes are ext4-name only.
run_ext_suite ext4 /dev/vde /ext_mx/ext2 noseed
run_ext_suite ext2 /dev/vde /ext_mx/ext2 noseed
run_ext_suite ext4 /dev/vdf /ext_mx/ext2_i128 noseed
run_ext_suite ext2 /dev/vdf /ext_mx/ext2_i128 noseed
run_ext_suite ext4 /dev/vdg /ext_mx/ext2_i128_nori noseed
run_ext_suite ext2 /dev/vdg /ext_mx/ext2_i128_nori noseed
run_ext_suite ext4 /dev/vdh /ext_mx/ext4_journal noseed
run_ext_suite ext4 /dev/vdi /ext_mx/ext4_noextents noseed
run_ext_suite ext2 /dev/vdi /ext_mx/ext4_noextents noseed
run_ext_suite ext4 /dev/vdd /ext_mx/ext4_seeded seed
# Negative cells. The ext2 name refuses extent/journaled volumes (Linux's
# IS_EXT2_SB rule); both names refuse the malformed images and the
# unsupported-ro-compat (metadata_csum) image.
expect_mount_rejected ext2 /dev/vdh /ext_mx/ext4_journal
expect_mount_rejected ext4 /dev/vdj /ext_mx/neg_nofiletype
expect_mount_rejected ext2 /dev/vdj /ext_mx/neg_nofiletype
expect_mount_rejected ext4 /dev/vdk /ext_mx/neg_rev0
expect_mount_rejected ext2 /dev/vdk /ext_mx/neg_rev0
expect_mount_rejected ext4 /dev/vdl /ext_mx/neg_metadata_csum
expect_mount_rejected ext2 /dev/vdl /ext_mx/neg_metadata_csum
echo "All ext matrix fs test passed."

echo "Start fdatasync test......"
test_fdatasync
echo "All fdatasync test passed."

echo "Start mount bind file test......"
test_mount_bind_file
echo "All mount bind file test passed."

./getcwd/getcwd

./inotify/inotify_align
./inotify/inotify_o_path
./inotify/inotify_poll
./inotify/inotify_unlink

./isolation/chroot
./isolation/pivot_root

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

./statx/btime

./symlink/symlink

./sync/sync

./tmpfile/tmpfile

./utimensat/utimensat
