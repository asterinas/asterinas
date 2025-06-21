#!/bin/sh

# SPDX-License-Identifier: MPL-2.0
# Entrypoint for the benchmark VM

set -e

BENCHMARK_ROOT="/benchmark"
READY_MESSAGE="The VM is ready for the benchmark."

BENCHMARK_NAME=$1
SYSTEM="${2:-asterinas}"
echo "Running benchmark: ${BENCHMARK_NAME} on ${SYSTEM}"

print_help() {
    echo "Usage: $0 <benchmark_name> <system_type>"
    echo "  benchmark_name: The name of the benchmark to run."
    echo "  system_type: The type of system to run the benchmark on. 'linux' or 'asterinas'."
}

# Validate arguments
check_benchmark_name() {
    if [ -z "${BENCHMARK_NAME}" ] || [ -z "${SYSTEM}" ]; then
        echo "Error: Invalid arguments."
        print_help
        exit 1
    fi

    local full_path="${BENCHMARK_ROOT}/${BENCHMARK_NAME}"

    if ! [ -d "${full_path}" ]; then
        echo "Directory '${BENCHMARK_NAME}' does not exist in the benchmark directory."
        print_help
        exit 1
    fi
}

prepare_system() {
    if [ ! -d /tmp ]; then
        mkdir /tmp
    fi

    # System-specific preparation
    if [ "$SYSTEM" = "linux" ]; then
        # Mount necessary fs
        mount -t devtmpfs devtmpfs /dev
        # Enable network
        ip link set lo up
        ip link set eth0 up
        ifconfig eth0 10.0.2.15
        # Mount ext2
        mount -t ext2 /dev/vda /ext2
    elif [ "$SYSTEM" = "asterinas" ]; then
        # Asterinas-specific preparation (if any)
        :
    else
        echo "Error: Unknown system type. Please set SYSTEM to 'linux' or 'asterinas'."
        exit 1
    fi
}

main() {
    # Check if the benchmark name is valid  
    check_benchmark_name

    # Prepare the system
    prepare_system
    
    # Message to notify the host script. It must align with the READY_MESSAGE in host_guest_bench_runner.sh.
    # DO NOT REMOVE THIS LINE!!!
    echo "${READY_MESSAGE}"

    # Run the benchmark
    BENCHMARK_SCRIPT=${BENCHMARK_ROOT}/${BENCHMARK_NAME}/run.sh
    chmod +x ${BENCHMARK_SCRIPT}
    ${BENCHMARK_SCRIPT}

    # Shutdown explicitly if running on Linux
    if [ "$SYSTEM" = "linux" ]; then
        poweroff -f
    fi
}

main "$@"
