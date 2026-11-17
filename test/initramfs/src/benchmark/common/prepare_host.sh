#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e
set -o pipefail

# Set BENCHMARK_ROOT to the parent directory of the current directory if it is not set
BENCHMARK_ROOT="${BENCHMARK_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." &>/dev/null && pwd)}"
# Set the log file
LINUX_OUTPUT="${BENCHMARK_ROOT}/linux_output.txt"
ASTER_OUTPUT="${BENCHMARK_ROOT}/aster_output.txt"
# Dependencies for Linux
LINUX_DEPENDENCIES_DIR="/opt/linux_binary_cache"
LINUX_KERNEL_OVERRIDE="${LINUX_KERNEL:+true}"
LINUX_KERNEL="${LINUX_KERNEL:-${LINUX_DEPENDENCIES_DIR}/vmlinuz}"
LINUX_KERNEL_VERSION="6.16.0"
LINUX_MODULES_DIR="${BENCHMARK_ROOT}/../build/initramfs/lib/modules/${LINUX_KERNEL_VERSION}/kernel"
WGET_SCRIPT="${BENCHMARK_ROOT}/../../../tools/atomic_wget.sh"

# Prepare Linux kernel and modules
prepare_libs() {
    if [[ "${BENCH_FILESYSTEM:-}" == erofs ]]; then
        if [[ "$LINUX_KERNEL_OVERRIDE" == true ]]; then
            echo "Error: EROFS fio uses the pinned Nix Linux kernel and its matching modules; unset LINUX_KERNEL" >&2
            return 1
        fi
        local kernel_dir
        kernel_dir=$(nix-build "${BENCHMARK_ROOT}/../../nix" --argstr target x86_64 \
            -A benchmark.linux-kernel --no-out-link)
        LINUX_KERNEL="$kernel_dir/bzImage"
        return
    fi
    mkdir -p "${LINUX_DEPENDENCIES_DIR}"

    # Array of files to download and their URLs
    declare -A files=(
        ["${LINUX_KERNEL}"]="https://raw.githubusercontent.com/asterinas/linux_binary_cache/8e6e8ad382aac7e605d7a058811d09bce478cd7d/${LINUX_KERNEL_VERSION}/vmlinuz"
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
    local benchmark="$1"
    if [[ -z "${benchmark}" ]]; then
        echo "Error: benchmark name is required to prepare the Linux filesystem." >&2
        return 1
    fi

    if [[ "${BENCH_FILESYSTEM:-}" == erofs ]]; then
        local initramfs_dir="${BENCHMARK_ROOT}/../../"
        EROFS_FIO_WORK_DIR="${EROFS_FIO_WORK_DIR:-${initramfs_dir}/build/erofs-fio/work}"
        mkdir -p "$EROFS_FIO_WORK_DIR"
        EROFS_FIO_WORK_DIR=$(cd "$EROFS_FIO_WORK_DIR" && pwd)
        make -C "$initramfs_dir" erofs-fio-image "EROFS_FIO_PROFILE=$EROFS_FIO_PROFILE" \
            "EROFS_FIO_WORK_DIR=$EROFS_FIO_WORK_DIR" 2>&1 | tee -a "$BENCH_OUTPUT_DIR/prepare_output.txt"
        EROFS_FIO_IMAGE=$(readlink -f "$EROFS_FIO_WORK_DIR/$EROFS_FIO_PROFILE.img")
        EROFS_FIO_FILE_BYTES=$(stat -c %s "$EROFS_FIO_WORK_DIR/root/silesia.tar")
        # QEMU drive options are comma-separated and OSDK expands its argument string.
        if [[ "$EROFS_FIO_IMAGE" == *[[:space:],]* ]]; then
            echo "Error: EROFS fio image path cannot contain whitespace or commas" >&2
            return 1
        fi
        export EROFS_FIO_IMAGE EROFS_FIO_FILE_BYTES EROFS_FIO_WORK_DIR EROFS_FIO_PROFILE
        cp "$EROFS_FIO_WORK_DIR/dataset.txt" "$BENCH_OUTPUT_DIR/"
        cp "$EROFS_FIO_IMAGE.txt" "$BENCH_OUTPUT_DIR/image.txt"
        cat "$EROFS_FIO_IMAGE.sha256" >> "$BENCH_OUTPUT_DIR/image.txt"
        make initramfs "BENCHMARK=$benchmark"
        return
    fi

    # Disable unsupported ext2 features of Asterinas on Linux to ensure fairness
    mke2fs -F -O ^ext_attr -O ^resize_inode -O ^dir_index ${BENCHMARK_ROOT}/../../build/ext2.img
    make initramfs BENCHMARK=${benchmark}
}
