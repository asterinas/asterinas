#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e
set -o pipefail

# Ensure all dependencies are installed
command -v jq >/dev/null 2>&1 || { echo >&2 "jq is not installed. Aborting."; exit 1; }

# Script directory
BENCHMARK_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" &>/dev/null && pwd)"

# Source the prepare_host.sh script
source "${BENCHMARK_DIR}/common/prepare_host.sh"

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
    local benchmark_type="$2"
    local search_pattern="$3"
    local result_index="$4"

    local linux_output="${BENCHMARK_DIR}/linux_output.txt"
    local aster_output="${BENCHMARK_DIR}/aster_output.txt"
    local result_template="${BENCHMARK_DIR}/${benchmark}/result_template.json"
    local benchmark_name=$(basename "${benchmark}")
    local benchmark_root=$(dirname "${benchmark}")
    local result_file="result_${benchmark_name}.json"
    
    echo "Preparing libraries..."
    prepare_libs

    local asterinas_cmd="make run BENCHMARK=${benchmark} ENABLE_KVM=1 RELEASE_LTO=1 2>&1"
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
        -append 'console=ttyS0 rdinit=/benchmark/common/bench_runner.sh ${benchmark} linux mitigations=off hugepages=0 transparent_hugepage=never quiet' \
        -netdev user,id=net01,hostfwd=tcp::5201-:5201,hostfwd=tcp::8080-:8080 \
        -device virtio-net-pci,netdev=net01,disable-legacy=on,disable-modern=off \
        -nographic \
        2>&1"
    case "${benchmark_type}" in
        "guest_only")
            echo "Running benchmark ${benchmark} on Asterinas..."
            eval "$asterinas_cmd" | tee ${aster_output}
            prepare_fs
            echo "Running benchmark ${benchmark} on Linux..."
            eval "$linux_cmd" | tee ${linux_output}
            ;;
        "host_guest")
            echo "Running benchmark ${benchmark} on host and guest..."
            bash "${BENCHMARK_DIR}/common/host_guest_bench_runner.sh" \
                "${BENCHMARK_DIR}/${benchmark}" \
                "${asterinas_cmd}" \
                "${linux_cmd}" \
                "${aster_output}" \
                "${linux_output}"
            ;;
        "guest-guest")
            echo "Running benchmark ${benchmark} between guests..."
            echo "TODO"
            exit 1
            ;;
        *)
            echo "Error: Unknown benchmark type '${benchmark_type}'" >&2
            exit 1
            ;;
    esac

    echo "Parsing results..."
    parse_results "$benchmark" "$search_pattern" "$result_index" "$linux_output" "$aster_output" "$result_template" "$result_file"

    echo "Cleaning up..."
    rm -f "${linux_output}"
    rm -f "${aster_output}"
}

# Main

BENCHMARK="$1"
if [ -z "$2" ] || [ "$2" = "null" ]; then
    BENCHMARK_TYPE="guest_only"
else
    BENCHMARK_TYPE="$2"
fi

echo "Running benchmark ${BENCHMARK}..."
pwd
if [ ! -d "$BENCHMARK_DIR/$BENCHMARK" ]; then
    echo "Error: Benchmark directory not found" >&2
    exit 1
fi

search_pattern=$(jq -r '.search_pattern' "$BENCHMARK_DIR/$BENCHMARK/config.json")
result_index=$(jq -r '.result_index' "$BENCHMARK_DIR/$BENCHMARK/config.json")

run_benchmark "$BENCHMARK" "$BENCHMARK_TYPE" "$search_pattern" "$result_index"

echo "Benchmark completed successfully."
