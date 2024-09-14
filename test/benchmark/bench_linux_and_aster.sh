#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e
set -o pipefail

# Ensure all dependencies are installed
command -v jq >/dev/null 2>&1 || { echo >&2 "jq is not installed. Aborting."; exit 1; }

# Script directory
BENCHMARK_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" &>/dev/null && pwd)"
# Dependencies for Linux
LINUX_DEPENDENCIES_DIR="/opt/linux_binary_cache"
LINUX_KERNEL="${LINUX_DEPENDENCIES_DIR}/vmlinuz"
LINUX_KERNEL_VERSION="5.15.0-105-generic"
LINUX_MODULES_DIR="${BENCHMARK_DIR}/../build/initramfs/lib/modules/${LINUX_KERNEL_VERSION}/kernel"
# Atomic wget script
WGET_SCRIPT="${BENCHMARK_DIR}/../../tools/atomic_wget.sh"

# Prepare Linux kernel and modules
prepare_libs() {
    # Download the Linux kernel and modules
    mkdir -p "${LINUX_DEPENDENCIES_DIR}"

    if [ ! -f "${LINUX_KERNEL}" ]; then
        echo "Downloading the Linux kernel image..."
        ${WGET_SCRIPT} "${LINUX_KERNEL}" "https://raw.githubusercontent.com/asterinas/linux_binary_cache/8a5b6fd/vmlinuz-${LINUX_KERNEL_VERSION}" || {
            echo "Failed to download the Linux kernel image."
            exit 1
        }
    fi
    if [ ! -f "${LINUX_DEPENDENCIES_DIR}/virtio_blk.ko" ]; then
        echo "Downloading the virtio_blk kernel module..."
        ${WGET_SCRIPT} "${LINUX_DEPENDENCIES_DIR}/virtio_blk.ko" "https://raw.githubusercontent.com/asterinas/linux_binary_cache/8a5b6fd/kernel/drivers/block/virtio_blk.ko" || {
            echo "Failed to download the Linux kernel module."
            exit 1
        }
    fi
    # Copy the kernel modules to the initramfs directory
    if [ ! -f "${LINUX_MODULES_DIR}/drivers/block/virtio_blk.ko" ]; then
        mkdir -p "${LINUX_MODULES_DIR}/drivers/block"
        cp ${LINUX_DEPENDENCIES_DIR}/virtio_blk.ko "${LINUX_MODULES_DIR}/drivers/block/virtio_blk.ko" 
    fi
}

# Prepare fs for Linux
prepare_fs() {
    # Disable unsupported ext2 features of Asterinas on Linux to ensure fairness
    mke2fs -F -O ^ext_attr -O ^resize_inode -O ^dir_index ${BENCHMARK_DIR}/../build/ext2.img
    make initramfs
}

# Parse the results from the benchmark output
parse_results() {
    local benchmark="$1"
    local search_pattern="$2"
    local result_index="$3"
    local linux_output="$4"
    local aster_output="$5"
    local result_template="$6"
    local result_file="$7"

    local linux_result aster_result
    linux_result=$(awk "/${search_pattern}/ {result=\$$result_index} END {print result}" "${linux_output}" | tr -d '\r')
    aster_result=$(awk "/${search_pattern}/ {result=\$$result_index} END {print result}" "${aster_output}" | tr -d '\r')
    
    if [ -z "${linux_result}" ] || [ -z "${aster_result}" ]; then
        echo "Error: Failed to parse the results from the benchmark output" >&2
        exit 1
    fi

    echo "Updating the result template with extracted values..."
    jq --arg linux_result "${linux_result}" --arg aster_result "${aster_result}" \
        '(.[] | select(.extra == "linux_result") | .value) |= $linux_result |
         (.[] | select(.extra == "aster_result") | .value) |= $aster_result' \
        "${result_template}" > "${result_file}"
}

# Run the benchmark on Linux and Asterinas
run_benchmark() {
    local benchmark="$1"
    local search_pattern="$2"
    local result_index="$3"

    local linux_output="${BENCHMARK_DIR}/linux_output.txt"
    local aster_output="${BENCHMARK_DIR}/aster_output.txt"
    local result_template="${BENCHMARK_DIR}/${benchmark}/result_template.json"
    local benchmark_name=$(basename "${benchmark}")
    local result_file="result_${benchmark_name}.json"
    
    echo "Preparing libraries..."
    prepare_libs

    local asterinas_cmd="make run BENCHMARK=${benchmark} ENABLE_KVM=1 RELEASE_LTO=1 2>&1 | tee ${aster_output}"
    echo "Running benchmark ${benchmark} on Asterinas..."
    eval "$asterinas_cmd"

    prepare_fs
    local linux_cmd="/usr/local/qemu/bin/qemu-system-x86_64 \
        --no-reboot \
        -smp 1 \
        -m 8G \
        -machine q35,kernel-irqchip=split \
        -cpu Icelake-Server,-pcid,+x2apic \
        --enable-kvm \
        -kernel ${LINUX_KERNEL} \
        -initrd ${BENCHMARK_DIR}/../build/initramfs.cpio.gz \
        -drive if=none,format=raw,id=x0,file=${BENCHMARK_DIR}/../build/ext2.img \
        -device virtio-blk-pci,bus=pcie.0,addr=0x6,drive=x0,serial=vext2,disable-legacy=on,disable-modern=off,queue-size=64,num-queues=1,config-wce=off,request-merging=off,write-cache=off,backend_defaults=off,discard=off,event_idx=off,indirect_desc=off,ioeventfd=off,queue_reset=off \
        -append 'console=ttyS0 rdinit=/benchmark/common/bench_runner.sh ${benchmark} linux mitigations=off hugepages=0 transparent_hugepage=never' \
        -nographic \
        2>&1 | tee ${linux_output}"
    echo "Running benchmark ${benchmark} on Linux..."
    eval "$linux_cmd"

    echo "Parsing results..."
    parse_results "$benchmark" "$search_pattern" "$result_index" "$linux_output" "$aster_output" "$result_template" "$result_file"

    echo "Cleaning up..."
    rm -f "${linux_output}"
    rm -f "${aster_output}"
}

# Main

BENCHMARK="$1"

echo "Running benchmark ${BENCHMARK}..."
pwd
if [ ! -d "$BENCHMARK_DIR/$BENCHMARK" ]; then
    echo "Error: Benchmark directory not found" >&2
    exit 1
fi

search_pattern=$(jq -r '.search_pattern' "$BENCHMARK_DIR/$BENCHMARK/config.json")
result_index=$(jq -r '.result_index' "$BENCHMARK_DIR/$BENCHMARK/config.json")

run_benchmark "$BENCHMARK" "$search_pattern" "$result_index"

echo "Benchmark completed successfully."
