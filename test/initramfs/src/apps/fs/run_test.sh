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

test_ext2() {
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

echo "Start ext2 fs test......"
test_ext2 "/ext2" "test_file.txt"
./ext2/mknod
./ext2/unix_socket
echo "All ext2 fs test passed."

echo "Start fdatasync test......"
test_fdatasync
echo "All fdatasync test passed."

echo "Start mount bind file test......"
test_mount_bind_file
echo "All mount bind file test passed."

./inotify/inotify_align
./inotify/inotify_poll
./inotify/inotify_unlink

./overlayfs/ovl_test

./procfs/dentry_cache
./procfs/pid_mem

./pseudofs/memfd_access_err
./pseudofs/pseudo_dentry
./pseudofs/pseudo_inode
./pseudofs/pseudo_mount
