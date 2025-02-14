#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e
set -o pipefail

# Ensure all dependencies are installed
command -v jq >/dev/null 2>&1 || { echo >&2 "jq is not installed. Aborting."; exit 1; }

# Set up paths
BENCHMARK_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" &>/dev/null && pwd)"
source "${BENCHMARK_ROOT}/common/prepare_host.sh"
RESULT_TEMPLATE="${BENCHMARK_ROOT}/result_template.json"

# Parse benchmark results
parse_raw_results() {
    local search_pattern="$1"
    local nth_occurrence="$2"
    local result_index="$3"
    local result_file="$4"

    # Extract and sanitize numeric results
    local linux_result aster_result
    linux_result=$(awk "/${search_pattern}/ {print \$$result_index}" "${LINUX_OUTPUT}" | tr -d '\r' | sed 's/[^0-9.]*//g' | sed -n "${nth_occurrence}p")
    aster_result=$(awk "/${search_pattern}/ {print \$$result_index}" "${ASTER_OUTPUT}" | tr -d '\r' | sed 's/[^0-9.]*//g' | sed -n "${nth_occurrence}p")

    # Ensure both results are valid
    if [ -z "${linux_result}" ] || [ -z "${aster_result}" ]; then
        echo "Error: Failed to parse the results from the benchmark output" >&2
        exit 1
    fi

    # Write the results into the template
    jq --arg linux_result "${linux_result}" --arg aster_result "${aster_result}" \
        '(.[] | select(.extra == "linux_result") | .value) |= $linux_result |
         (.[] | select(.extra == "aster_result") | .value) |= $aster_result' \
        "${RESULT_TEMPLATE}" > "${result_file}"
}

# Generate a new result template based on unit and legend
generate_template() {
    local unit="$1"
    local legend="$2"

    # Replace placeholders with actual system names
    local linux_legend=${legend//"{system}"/"Linux"}
    local asterinas_legend=${legend//"{system}"/"Asterinas"}

    # Generate the result template JSON
    jq -n --arg linux "$linux_legend" --arg aster "$asterinas_legend" --arg unit "$unit" '[
        { "name": $linux, "unit": $unit, "value": 0, "extra": "linux_result" },
        { "name": $aster, "unit": $unit, "value": 0, "extra": "aster_result" }
    ]' > "${RESULT_TEMPLATE}"
}

# Extract the result file path based on benchmark location
extract_result_file() {
    local bench_result="$1"
    local relative_path="${bench_result#*/benchmark/}"
    local first_dir="${relative_path%%/*}"
    local filename=$(basename "$bench_result")

    # Handle different naming conventions for result files
    if [[ "$filename" == bench_* ]]; then
        local second_part=$(dirname "$bench_result" | awk -F"/benchmark/$first_dir/" '{print $2}' | cut -d'/' -f1)
        echo "result_${first_dir}-${second_part}.json"
    else
        echo "result_${relative_path//\//-}"
    fi
}

# Run the specified benchmark with optional scheme
run_benchmark() {
    local benchmark="$1"
    local run_mode="$2"
    local aster_scheme="$3"
    local smp="$4"

    echo "Preparing libraries..."
    prepare_libs

    # Set up Asterinas scheme if specified (Default: iommu)
    local aster_scheme_cmd="SCHEME=iommu"
    if [ -n "$aster_scheme" ]; then
        if [ "$aster_scheme" != "null" ]; then
            aster_scheme_cmd="SCHEME=${aster_scheme}"
        else
            aster_scheme_cmd=""
        fi
    fi

    # Prepare commands for Asterinas and Linux
    local asterinas_cmd="make run BENCHMARK=${benchmark} ${aster_scheme_cmd} SMP=${smp} ENABLE_KVM=1 RELEASE_LTO=1 NETDEV=tap VHOST=on 2>&1"
    local linux_cmd="/usr/local/qemu/bin/qemu-system-x86_64 \
        --no-reboot \
        -smp ${smp} \
        -m 8G \
        -machine q35,kernel-irqchip=split \
        -cpu Icelake-Server,-pcid,+x2apic \
        --enable-kvm \
        -kernel ${LINUX_KERNEL} \
        -initrd ${BENCHMARK_ROOT}/../build/initramfs.cpio.gz \
        -drive if=none,format=raw,id=x0,file=${BENCHMARK_ROOT}/../build/ext2.img \
        -device virtio-blk-pci,bus=pcie.0,addr=0x6,drive=x0,serial=vext2,disable-legacy=on,disable-modern=off,queue-size=64,num-queues=1,request-merging=off,backend_defaults=off,discard=off,write-zeroes=off,event_idx=off,indirect_desc=off,queue_reset=off \
        -append 'console=ttyS0 rdinit=/benchmark/common/bench_runner.sh ${benchmark} linux mitigations=off hugepages=0 transparent_hugepage=never quiet' \
        -netdev tap,id=net01,script=${BENCHMARK_ROOT}/../../tools/net/qemu-ifup.sh,downscript=${BENCHMARK_ROOT}/../../tools/net/qemu-ifdown.sh,vhost=on \
        -device virtio-net-pci,netdev=net01,disable-legacy=on,disable-modern=off,csum=off,guest_csum=off,ctrl_guest_offloads=off,guest_tso4=off,guest_tso6=off,guest_ecn=off,guest_ufo=off,host_tso4=off,host_tso6=off,host_ecn=off,host_ufo=off,mrg_rxbuf=off,ctrl_vq=off,ctrl_rx=off,ctrl_vlan=off,ctrl_rx_extra=off,guest_announce=off,ctrl_mac_addr=off,host_ufo=off,guest_uso4=off,guest_uso6=off,host_uso=off \
        -nographic \
        2>&1"

    # Run the benchmark depending on the mode
    case "${run_mode}" in
        "guest_only")
            echo "Running benchmark ${benchmark} on Asterinas..."
            eval "$asterinas_cmd" | tee ${ASTER_OUTPUT}
            prepare_fs
            echo "Running benchmark ${benchmark} on Linux..."
            eval "$linux_cmd" | tee ${LINUX_OUTPUT}
            ;;
        "host_guest")
            echo "Running benchmark ${benchmark} on host and guest..."
            bash "${BENCHMARK_ROOT}/common/host_guest_bench_runner.sh" \
                "${BENCHMARK_ROOT}/${benchmark}" \
                "${asterinas_cmd}" \
                "${linux_cmd}" \
                "${ASTER_OUTPUT}" \
                "${LINUX_OUTPUT}"
            ;;
        *)
            echo "Error: Unknown benchmark type '${run_mode}'" >&2
            exit 1
            ;;
    esac
}

# Parse the benchmark configuration
parse_results() {
    local bench_result="$1"

    local search_pattern=$(jq -r '.result_extraction.search_pattern // empty' "$bench_result")
    local nth_occurrence=$(jq -r '.result_extraction.nth_occurrence // 1' "$bench_result")
    local result_index=$(jq -r '.result_extraction.result_index // empty' "$bench_result")
    local unit=$(jq -r '.chart.unit // empty' "$bench_result")
    local legend=$(jq -r '.chart.legend // {system}' "$bench_result")

    generate_template "$unit" "$legend"
    parse_raw_results "$search_pattern" "$nth_occurrence" "$result_index" "$(extract_result_file "$bench_result")"
}

# Clean up temporary files
cleanup() {
    echo "Cleaning up..."
    rm -f "${LINUX_OUTPUT}" "${ASTER_OUTPUT}" "${RESULT_TEMPLATE}"
}

# Main function to coordinate the benchmark run
main() {
    local benchmark="$1"
    if [[ -z "${BENCHMARK_ROOT}/${benchmark}" ]]; then
        echo "Error: No benchmark specified" >&2
        exit 1
    fi
    echo "Running benchmark $benchmark..."

    # Determine the run mode (host-only or host-guest)
    local run_mode="guest_only"
    [[ -f "${BENCHMARK_ROOT}/${benchmark}/host.sh" ]] && run_mode="host_guest"

    local bench_result="${BENCHMARK_ROOT}/${benchmark}/bench_result.json"
    local aster_scheme
    if [[ -f "$bench_result" ]]; then
        aster_scheme=$(jq -r '.runtime_config.aster_scheme // ""' "$bench_result")
    else
        for job in "${BENCHMARK_ROOT}/${benchmark}"/bench_results/*; do
            [[ -f "$job" ]] && aster_scheme=$(jq -r '.runtime_config.aster_scheme // ""' "$job") && break
        done
    fi

    local smp
    if [[ -f "$bench_result" ]]; then
        smp=$(jq -r '.runtime_config.smp // 1' "$bench_result")
    else
        for job in "${BENCHMARK_ROOT}/${benchmark}"/bench_results/*; do
            [[ -f "$job" ]] && smp=$(jq -r '.runtime_config.smp // 1' "$job") && break
        done
    fi

    # Run the benchmark
    run_benchmark "$benchmark" "$run_mode" "$aster_scheme" "$smp"

    # Parse results if benchmark configuration exists
    if [[ -f "$bench_result" ]]; then
        parse_results "$bench_result"
    else
        for job in "${BENCHMARK_ROOT}/${benchmark}"/bench_results/*; do
            [[ -f "$job" ]] && parse_results "$job"
        done
    fi

    # Cleanup temporary files
    cleanup
    echo "Benchmark completed successfully."
}

main "$@"
