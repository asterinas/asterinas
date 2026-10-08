#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e
set -o pipefail

# Ensure all dependencies are installed
if ! command -v yq >/dev/null 2>&1; then
    echo >&2 "Error: missing required tool: yq"
    exit 1
fi
if ! command -v jq >/dev/null 2>&1; then
    echo >&2 "Error: missing required tool: jq"
    exit 1
fi

# Set up paths
BENCHMARK_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" &>/dev/null && pwd)"
source "${BENCHMARK_ROOT}/common/prepare_host.sh"
source "${BENCHMARK_ROOT}/fio/common/parse_result.sh"
RESULT_TEMPLATE="${BENCHMARK_ROOT}/result_template.json"

# Extract and sanitize numeric results.
extract_raw_result() {
    local search_pattern="$1" result_index="$2" output="$3"

    awk -v pattern="$search_pattern" -v field="$result_index" '
        $0 ~ pattern {
            value = $field
            gsub(/\r/, "", value)
            gsub(/[^0-9.]/, "", value)
            print value
        }
    ' "$output"
}

# Parse benchmark results
parse_raw_results() {
    local search_pattern="$1"
    local nth_occurrence="$2"
    local result_index="$3"
    local result_file="$4"
    local parser="$5"
    local direction="$6"

    # Extract and sanitize numeric results
    local linux_result aster_result
    case "$parser" in
        text)
            linux_result=$(extract_raw_result "$search_pattern" "$result_index" "$LINUX_OUTPUT" | sed -n "${nth_occurrence}p")
            aster_result=$(extract_raw_result "$search_pattern" "$result_index" "$ASTER_OUTPUT" | sed -n "${nth_occurrence}p")
            ;;
        fio_json)
            linux_result=$(extract_fio_result "$LINUX_OUTPUT" "$direction")
            aster_result=$(extract_fio_result "$ASTER_OUTPUT" "$direction")
            ;;
        *)
            echo "Error: Unknown result parser '$parser'" >&2
            return 1
            ;;
    esac

    # Ensure both results are valid
    if [ -z "${linux_result}" ] || [ -z "${aster_result}" ]; then
        echo "Error: Failed to parse the results from the benchmark output" >&2
        exit 1
    fi

    # Write the results into the template
    yq --arg linux_result "${linux_result}" --arg aster_result "${aster_result}" \
        '(.[] | select(.extra == "linux_result") | .value) |= $linux_result |
         (.[] | select(.extra == "aster_result") | .value) |= $aster_result' \
        "${RESULT_TEMPLATE}" > "${result_file}"
    echo "Results written to ${result_file}"
}

# Generate a new result template based on unit and legend
generate_template() {
    local unit="$1"
    local legend="$2"

    # Replace placeholders with actual system names
    local linux_legend=${legend//"{system}"/"Linux"}
    local asterinas_legend=${legend//"{system}"/"Asterinas"}

    # Generate the result template JSON
    yq -n --arg linux "$linux_legend" --arg aster "$asterinas_legend" --arg unit "$unit" '[
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
        local result_file="result_${relative_path//\//-}"
        echo "${result_file/.yaml/.json}"
    fi
}

# Linux is launched directly rather than through OSDK, so manage its daemon here.
run_linux_vm() (
    local virtiofs="$1" mem="$2" tag="$3"
    shift 3
    local linux_cmd=("$@")

    if [[ "$virtiofs" == "on" ]]; then
        local work_dir daemon_pid
        work_dir=$(mktemp -d /tmp/asterinas-bench-virtiofs-XXXXXX)
        stop_virtiofsd() {
            kill "$daemon_pid" 2>/dev/null || true
            wait "$daemon_pid" 2>/dev/null || true
            rm -rf -- "$work_dir"
        }
        "${BENCHMARK_ROOT}/../../../../tools/run_virtiofsd.sh" --work-dir "$work_dir" &
        daemon_pid=$!
        trap stop_virtiofsd EXIT
        trap 'exit 130' INT
        trap 'exit 143' TERM

        local attempt
        for ((attempt = 0; attempt < 100; attempt++)); do
            [[ -S "$work_dir/vfs.sock" ]] && break
            if ! kill -0 "$daemon_pid" 2>/dev/null; then
                cat "$work_dir/virtiofsd.log" >&2
                exit 1
            fi
            sleep 0.1
        done
        if [[ ! -S "$work_dir/vfs.sock" ]]; then
            echo "Error: Timed out waiting for virtiofsd." >&2
            exit 1
        fi

        linux_cmd+=(
            -object "memory-backend-memfd,id=mem0,size=${mem},share=on"
            -numa "node,memdev=mem0"
            -chardev "socket,id=char0,path=${work_dir}/vfs.sock"
            -device "vhost-user-fs-pci,chardev=char0,tag=${tag}"
        )
    fi

    "${linux_cmd[@]}"
)

# Run the specified benchmark with runtime configurations
run_benchmark() {
    local benchmark="$1"
    local run_mode="$2"
    local runtime_configs_str="$3" # String with key=value pairs, one per line

    echo "Preparing libraries..."
    prepare_libs

    # Default values
    local smp_val=1
    local mem_val="8G"
    local aster_scheme_cmd_part="SCHEME=iommu" # Default scheme
    local virtiofs_val="off"
    local virtiofs_tag="aster-virtiofs"

    # Process runtime_configs_str to override defaults and gather extra args
    while IFS='=' read -r key value; do
         if [[ -z "$key" ]]; then continue; fi # Skip empty lines/keys
         case "$key" in
             "smp")
                 smp_val="$value"
                 ;;
             "mem")
                 mem_val="$value"
                 ;;
             "aster_scheme")
                 if [[ "$value" == "null" ]]; then
                     aster_scheme_cmd_part="" # Remove default SCHEME=iommu
                 else
                     aster_scheme_cmd_part="SCHEME=${value}" # Override default
                 fi
                 ;;
             "virtiofs")
                 if [[ "$value" != "on" && "$value" != "off" ]]; then
                     echo "Error: Invalid virtiofs setting '$value'" >&2
                     exit 1
                 fi
                 virtiofs_val="$value"
                 ;;
             "virtiofs_tag")
                 virtiofs_tag="$value"
                 ;;
             *)
                 echo "Warning: Unknown runtime configuration key '$key'" >&2
                 exit 1
                 ;;
         esac
     done <<< "$runtime_configs_str"

    # Prepare commands for Asterinas and Linux using arrays
    local asterinas_cmd_arr=(make run_kernel "BENCHMARK=${benchmark}")
    # Add scheme part only if it's not empty and the platform is not TDX (OSDK doesn't support multiple SCHEME)
    [[ -n "$aster_scheme_cmd_part" && "$platform" != "tdx" ]] && asterinas_cmd_arr+=("$aster_scheme_cmd_part")
    asterinas_cmd_arr+=(
        "SMP=${smp_val}"
        "MEM=${mem_val}"
        ENABLE_KVM=1
        RELEASE_LTO=1
        NETDEV=tap
        VHOST=on
        "VIRTIOFS=${virtiofs_val}"
        "VIRTIOFS_TAG=${virtiofs_tag}"
    )
    if [[ "$platform" == "tdx" ]]; then
        asterinas_cmd_arr+=(INTEL_TDX=1)
    fi

    local linux_cmd_arr=(
        qemu-system-x86_64
        --no-reboot
        -smp "${smp_val}"
        -m "${mem_val}"
        -machine q35,kernel-irqchip=split
        --enable-kvm
        -kernel "${LINUX_KERNEL}"
        -initrd "${BENCHMARK_ROOT}/../../build/initramfs.cpio.gz"
        -drive "if=none,format=raw,id=x0,file=${BENCHMARK_ROOT}/../../build/ext2.img"
        -device "virtio-blk-pci,bus=pcie.0,addr=0x6,drive=x0,serial=vext2,disable-legacy=on,disable-modern=off,queue-size=64,num-queues=1,request-merging=off,backend_defaults=off,discard=off,write-zeroes=off,event_idx=off,indirect_desc=off,queue_reset=off"
        -drive "if=none,format=raw,id=nvme0n1,file=${BENCHMARK_ROOT}/../../build/nvme0n1.img"
        -device "nvme,drive=nvme0n1,serial=nvme0n1"
        -device "virtio-net-pci,netdev=net01,disable-legacy=on,disable-modern=off,csum=off,ctrl_guest_offloads=off,ctrl_mac_addr=off,ctrl_rx_extra=off,ctrl_rx=off,ctrl_vlan=off,ctrl_vq=off,event_idx=off,guest_announce=off,guest_csum=off,guest_ecn=off,guest_tso4=off,guest_tso6=off,guest_ufo=off,guest_uso4=off,guest_uso6=off,host_ecn=off,host_tso4=off,host_tso6=off,host_ufo=off,host_uso=off,indirect_desc=off,mrg_rxbuf=off,queue_reset=off"
        -append "console=ttyS0 rdinit=/benchmark/common/bench_runner.sh ${benchmark} linux mitigations=off hugepages=0 transparent_hugepage=never quiet"
        -netdev "tap,id=net01,script=${BENCHMARK_ROOT}/../../../../tools/net/qemu-ifup.sh,downscript=${BENCHMARK_ROOT}/../../../../tools/net/qemu-ifdown.sh,vhost=on"
        -nographic
    )
    if [[ "$platform" != "tdx" ]]; then
        linux_cmd_arr+=(
            -cpu Icelake-Server,-pcid,+x2apic
        )
    else
        linux_cmd_arr+=(
            -machine confidential-guest-support=tdx0
            -cpu host,-kvm-steal-time,pmu=off
            -bios "${OVMF_DIR:-/root/ovmf/release}/OVMF.fd"
            -nodefaults
            -serial stdio
            -object '{ "qom-type": "tdx-guest", "id": "tdx0", "sept-ve-disable": true, "quote-generation-socket": { "type": "vsock", "cid": "1", "port": "4050" } }'
        )
    fi

    # Run the benchmark depending on the mode
    case "${run_mode}" in
        "guest_only")
            echo "Running benchmark ${benchmark} on Asterinas..."
            # Execute directly from array, redirect stderr to stdout, then tee
            "${asterinas_cmd_arr[@]}" 2>&1 | tee "${ASTER_OUTPUT}"
            prepare_fs "$benchmark"
            echo "Running benchmark ${benchmark} on Linux..."
            # Execute directly from array, redirect stderr to stdout, then tee
            run_linux_vm "$virtiofs_val" "$mem_val" "$virtiofs_tag" "${linux_cmd_arr[@]}" 2>&1 | tee "${LINUX_OUTPUT}"
            ;;
        "host_guest")
            # Note: host_guest_bench_runner.sh expects commands as single strings.
            # We need to reconstruct the string representation for compatibility.
            # Use printf %q to quote arguments safely.
            local asterinas_cmd_str
            printf -v asterinas_cmd_str '%q ' "${asterinas_cmd_arr[@]}"
            local linux_cmd_str
            printf -v linux_cmd_str '%q ' "${linux_cmd_arr[@]}"

            echo "Running benchmark ${benchmark} on host and guest..."
            bash "${BENCHMARK_ROOT}/common/host_guest_bench_runner.sh" \
                "${BENCHMARK_ROOT}/${benchmark}" \
                "${asterinas_cmd_str}" \
                "${linux_cmd_str}" \
                "${ASTER_OUTPUT}" \
                "${LINUX_OUTPUT}" \
                "${benchmark}"
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

    local search_pattern nth_occurrence result_index unit legend parser direction
    search_pattern=$(yq -r '.result_extraction.search_pattern // empty' "$bench_result")
    nth_occurrence=$(yq -r '.result_extraction.nth_occurrence // 1' "$bench_result")
    result_index=$(yq -r '.result_extraction.result_index // empty' "$bench_result")
    unit=$(yq -r '.chart.unit // empty' "$bench_result")
    legend=$(yq -r '.chart.legend // {system}' "$bench_result")
    parser=$(yq -r '.result_extraction.parser // "text"' "$bench_result")
    direction=$(yq -r '.result_extraction.direction // empty' "$bench_result")

    if [[ "$parser" == "fio_json" && "$unit" != "MB/s" ]]; then
        echo "Error: The fio_json parser produces results in MB/s." >&2
        return 1
    fi

    generate_template "$unit" "$legend"
    parse_raw_results "$search_pattern" "$nth_occurrence" "$result_index" "$(extract_result_file "$bench_result")" "$parser" "$direction"
}

# Clean up temporary files
cleanup() {
    echo "Cleaning up..."
    rm -f "${LINUX_OUTPUT}" "${ASTER_OUTPUT}" "${RESULT_TEMPLATE}"
}

# Main function to coordinate the benchmark run
main() {
    local benchmark="$1"
    local platform="$2"

    if [[ -z "${BENCHMARK_ROOT}/${benchmark}" ]]; then
        echo "Error: No benchmark specified" >&2
        exit 1
    fi
    echo "Running benchmark $benchmark..."

    # Determine the run mode (host-only or host-guest)
    local run_mode="guest_only"
    [[ -f "${BENCHMARK_ROOT}/${benchmark}/host.sh" ]] && run_mode="host_guest"

    local bench_result="${BENCHMARK_ROOT}/${benchmark}/bench_result.yaml"
    local runtime_configs_str=""

    # Try reading from single result file first
    if [[ -f "$bench_result" ]]; then
        # Read runtime_config object, convert to key=value lines, ensuring value is string
        runtime_configs_str=$(yq -r '(.runtime_config // {}) | to_entries | .[] | .key + "=" + (.value | tostring)' "$bench_result")
    else
        # If not found, try reading from the first file in bench_results/ that has a non-empty runtime_config
        for job_yaml in "${BENCHMARK_ROOT}/${benchmark}"/bench_results/*; do
            if [[ -f "$job_yaml" ]]; then
                echo "Reading runtime configurations from $job_yaml..."
                # Read runtime_config object, convert to key=value lines, ensuring value is string
                runtime_configs_str=$(yq -r '(.runtime_config // {}) | to_entries | .[] | .key + "=" + (.value | tostring)' "$job_yaml")
                # Check if runtime_config was actually found and non-empty
                if [[ -n "$runtime_configs_str" ]]; then
                    break # Found it, stop looking
                fi
            fi
        done
    fi

    # Run the benchmark, passing the config string
    run_benchmark "$benchmark" "$run_mode" "$runtime_configs_str"

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
