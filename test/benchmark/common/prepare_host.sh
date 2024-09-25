#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e
set -o pipefail

# Set BENCHMARK_DIR to the parent directory of the current directory if it is not set
BENCHMARK_DIR="${BENCHMARK_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." &>/dev/null && pwd)}"
# Dependencies for Linux
LINUX_DEPENDENCIES_DIR="/opt/linux_binary_cache"
LINUX_KERNEL="${LINUX_DEPENDENCIES_DIR}/vmlinuz"
LINUX_KERNEL_VERSION="5.15.0-105-generic"
LINUX_MODULES_DIR="${BENCHMARK_DIR}/../build/initramfs/lib/modules/${LINUX_KERNEL_VERSION}/kernel"
WGET_SCRIPT="${BENCHMARK_DIR}/../../tools/atomic_wget.sh"

# Prepare Linux kernel and modules
prepare_libs() {
    # Download the Linux kernel and modules
    mkdir -p "${LINUX_DEPENDENCIES_DIR}"

    # Array of files to download and their URLs
    declare -A files=(
        ["${LINUX_KERNEL}"]="https://raw.githubusercontent.com/asterinas/linux_binary_cache/8a5b6fd/vmlinuz-${LINUX_KERNEL_VERSION}"
        ["${LINUX_DEPENDENCIES_DIR}/virtio_blk.ko"]="https://raw.githubusercontent.com/asterinas/linux_binary_cache/8a5b6fd/kernel/drivers/block/virtio_blk.ko"
        ["${LINUX_DEPENDENCIES_DIR}/virtio_net.ko"]="https://raw.githubusercontent.com/asterinas/linux_binary_cache/8a5b6fd/kernel/drivers/net/virtio_net.ko"
        ["${LINUX_DEPENDENCIES_DIR}/net_failover.ko"]="https://raw.githubusercontent.com/asterinas/linux_binary_cache/8a5b6fd/kernel/drivers/net/net_failover.ko"
        ["${LINUX_DEPENDENCIES_DIR}/failover.ko"]="https://raw.githubusercontent.com/asterinas/linux_binary_cache/8a5b6fd/kernel/net/core/failover.ko"
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

    # Copy the kernel modules to the initramfs directory
    if [ ! -f "${LINUX_MODULES_DIR}/drivers/block/virtio_blk.ko" ]; then
        mkdir -p "${LINUX_MODULES_DIR}/drivers/block"
        mkdir -p "${LINUX_MODULES_DIR}/drivers/net"
        mkdir -p "${LINUX_MODULES_DIR}/net/core"

        declare -A modules=(
            ["${LINUX_DEPENDENCIES_DIR}/virtio_blk.ko"]="${LINUX_MODULES_DIR}/drivers/block/virtio_blk.ko"
            ["${LINUX_DEPENDENCIES_DIR}/virtio_net.ko"]="${LINUX_MODULES_DIR}/drivers/net/virtio_net.ko"
            ["${LINUX_DEPENDENCIES_DIR}/net_failover.ko"]="${LINUX_MODULES_DIR}/drivers/net/net_failover.ko"
            ["${LINUX_DEPENDENCIES_DIR}/failover.ko"]="${LINUX_MODULES_DIR}/net/core/failover.ko"
        )

        for src in "${!modules[@]}"; do
            sudo cp "$src" "${modules[$src]}"
        done
    fi
}

# Prepare fs for Linux
prepare_fs() {
    # Disable unsupported ext2 features of Asterinas on Linux to ensure fairness
    mke2fs -F -O ^ext_attr -O ^resize_inode -O ^dir_index ${BENCHMARK_DIR}/../build/ext2.img
    make initramfs
}