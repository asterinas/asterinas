#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e
set -o pipefail

# Ensure all dependencies are installed
command -v jq >/dev/null 2>&1 || { echo >&2 "jq is not installed. Aborting."; exit 1; }

# Script directory
BENCHMARK_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" &>/dev/null && pwd)"
# Kernel image 
KERNEL_DIR="/root/dependency"
LINUX_KERNEL="${KERNEL_DIR}/vmlinuz"
# Atomic wget script
WGET_SCRIPT="${BENCHMARK_DIR}/../../tools/atomic_wget.sh"

# Generate entrypoint script for Linux cases
generate_entrypoint_script() {
    local benchmark="$1"
    local init_script=$(cat <<EOF
#!/bin/sh

echo "Running ${benchmark}"
chmod +x /benchmark/${benchmark}/run.sh
/benchmark/${benchmark}/run.sh

poweroff -f
EOF
)
    echo "$init_script"
}

# Run the benchmark on Linux and Asterinas
run_benchmark() {
    local benchmark="$1"
    local search_pattern="$2"
    local result_index="$3"

    local linux_output="${BENCHMARK_DIR}/linux_output.txt"
    local aster_output="${BENCHMARK_DIR}/aster_output.txt"
    local result_template="${BENCHMARK_DIR}/${benchmark}/result_template.json"
    local result_file="result_${benchmark}.json"

    # Entrypoint script for initramfs
    local initramfs_entrypoint_script="${BENCHMARK_DIR}/benchmark_entrypoint.sh"
    generate_entrypoint_script "${benchmark}" > "${initramfs_entrypoint_script}"
    chmod +x "${initramfs_entrypoint_script}"
        
    # TODO: enable nopti for Linux to make the comparison more fair
    local qemu_cmd="/usr/local/qemu/bin/qemu-system-x86_64 \
        --no-reboot \
        -smp 1 \
        -m 8G \
        -machine q35,kernel-irqchip=split \
        -cpu Icelake-Server,+x2apic \
        --enable-kvm \
        -kernel ${LINUX_KERNEL} \
        -initrd ${BENCHMARK_DIR}/../build/initramfs.cpio.gz \
        -append 'console=ttyS0 rdinit=/benchmark/benchmark_entrypoint.sh' \
        -nographic \
        2>&1 | tee ${linux_output}" 

    if [ ! -f "${LINUX_KERNEL}" ]; then
        echo "Downloading the Linux kernel image..."
        mkdir -p "${KERNEL_DIR}"
        ${WGET_SCRIPT} "${LINUX_KERNEL}" "https://raw.githubusercontent.com/asterinas/linux_kernel/9e66d28/vmlinuz-5.15.0-105-generic" 
    fi

    echo "Running benchmark ${benchmark} on Linux and Asterinas..."
    make run BENCHMARK=${benchmark} ENABLE_KVM=1 RELEASE=1 2>&1 | tee "${aster_output}"
    eval "$qemu_cmd"

    echo "Parsing results..."
    local linux_avg aster_avg
    linux_avg=$(awk "/${search_pattern}/{print \$$result_index}" "${linux_output}" | tr -d '\r')
    aster_avg=$(awk "/${search_pattern}/{print \$$result_index}" "${aster_output}" | tr -d '\r')

    if [ -z "${linux_avg}" ] || [ -z "${aster_avg}" ]; then
        echo "Error: Failed to parse the average value from the benchmark output" >&2
        exit 1
    fi

    echo "Updating the result template with average values..."
    jq --arg linux_avg "${linux_avg}" --arg aster_avg "${aster_avg}" \
        '(.[] | select(.extra == "linux_avg") | .value) |= $linux_avg |
         (.[] | select(.extra == "aster_avg") | .value) |= $aster_avg' \
        "${result_template}" > "${result_file}"

    echo "Cleaning up..."
    rm -f "${initramfs_entrypoint_script}"
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
