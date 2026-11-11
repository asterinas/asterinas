#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e
# Keep the same relative binary paths when invoked by fs/run_test.sh or alone.
cd "$(dirname "$0")/.."

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
    local device="$4"

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
    EXT_REMOUNT_PHASE=prepare ./ext/remount
    umount /ext-test
    sync
    umount "$source"
    mount -t "$suite_name" -o minixdf "$device" "$source"
    EXT_STATFS_DEVICE="$device" EXT_STATFS_PATH="$source" \
        EXT_STATFS_EXPECT=total ./ext/statfs_options
    mount --bind "$source" /ext-test
    EXT_REMOUNT_PHASE=verify ./ext/remount
    umount /ext-test
    umount "$source"
    mount -t "$suite_name" -o bsddf "$device" "$source"
    EXT_STATFS_DEVICE="$device" EXT_STATFS_PATH="$source" \
        EXT_STATFS_EXPECT=overhead ./ext/statfs_options
    echo "All $suite_name shared EXT tests and real remount passed."
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

run_ext_suite ext2 /ext2 no /dev/vda
mount --bind /ext2 /ext-test
run_test_list ./ext/ext2-only.tests
umount /ext-test
./ext/shared_block_device
echo "All ext2-only EXT tests passed."

mkdir -p /ext4
mount -t ext4 /dev/vdd /ext4
run_ext_suite ext4 /ext4 yes /dev/vdd
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

# This disposable image is separate from the shared suites and all host devices.
mkdir -p /ext-space
mount -t ext2 /dev/vdj /ext-space
EXT_STATFS_DEVICE=/dev/vdj EXT_STATFS_PATH=/ext-space \
    EXT_STATFS_EXPECT=2035 ./ext/statfs_options
./ext/fallocate_enospc
umount /ext-space
mount -t ext2 -o bsddf /dev/vdj /ext-space
EXT_STATFS_DEVICE=/dev/vdj EXT_STATFS_PATH=/ext-space \
    EXT_STATFS_EXPECT=2035 ./ext/statfs_options
umount /ext-space
mount -t ext2 -o minixdf /dev/vdj /ext-space
EXT_STATFS_DEVICE=/dev/vdj EXT_STATFS_PATH=/ext-space \
    EXT_STATFS_EXPECT=total ./ext/statfs_options
umount /ext-space
echo "All EXT regression tests passed."
