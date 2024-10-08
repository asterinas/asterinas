#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e
set -o pipefail

# Set BENCHMARK_DIR to the parent directory of the current directory if it is not set
BENCHMARK_DIR="${BENCHMARK_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." &>/dev/null && pwd)}"
# Dependencies for Linux
LINUX_DEPENDENCIES_DIR="/opt/linux_binary_cache"
LINUX_KERNEL="${LINUX_DEPENDENCIES_DIR}/vmlinuz"
LINUX_KERNEL_VERSION="5.15.0-105"
LINUX_MODULES_DIR="${BENCHMARK_DIR}/../build/initramfs/lib/modules/${LINUX_KERNEL_VERSION}/kernel"
WGET_SCRIPT="${BENCHMARK_DIR}/../../tools/atomic_wget.sh"

# Prepare Linux kernel and modules
prepare_libs() {
    # Download the Linux kernel and modules
    mkdir -p "${LINUX_DEPENDENCIES_DIR}"

    # Array of files to download and their URLs
    declare -A files=(
        ["${LINUX_KERNEL}"]="https://raw.githubusercontent.com/asterinas/linux_binary_cache/14598b6/vmlinuz-${LINUX_KERNEL_VERSION}"
    )

    # Download files if they don't exist
    for file in "${!files[@]}"; do
        if [ ! -f "$file" ]; then
            echo "Downloading ${file##*/}..."
            ${WGET_SCRIPT} "$file" "${files[$file]}" || {
                echo "Failed to download ${file##*/}."
                exit 1
            }
        fi
    done
}

# Prepare fs for Linux
prepare_fs() {
    # Disable unsupported ext2 features of Asterinas on Linux to ensure fairness
    mke2fs -F -O ^ext_attr -O ^resize_inode -O ^dir_index ${BENCHMARK_DIR}/../build/ext2.img
    make initramfs
}