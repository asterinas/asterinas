#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e
set -x

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
    fdatasync/fdatasync /
    rm -f /test_fdatasync.txt
    fdatasync/fdatasync /ext2
    rm -f /ext2/test_fdatasync.txt
    fdatasync/fdatasync /exfat
    rm -f /exfat/test_fdatasync.txt
}

echo "Start ext2 fs test......"
test_ext2 "/ext2" "test_file.txt"
echo "All ext2 fs test passed."

echo "Start fdatasync test......"
test_fdatasync
echo "All fdatasync test passed."

pipe/pipe_err
pipe/short_rw
epoll/epoll_err
epoll/poll_err
