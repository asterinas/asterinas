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

EXT2_DIR=/ext2
cd ${EXT2_DIR}

echo "Start ext2 fs test......"

# Test case for the big file feature
truncate -s 500M test_file.txt
check_file_size test_file.txt $((500 * 1024 * 1024))
truncate -s 2K test_file.txt
check_file_size test_file.txt $((2 * 1024))
sync

echo "All ext2 fs test passed."