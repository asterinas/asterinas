#!/bin/sh

# SPDX-License-Identifier: MPL-2.0
# Entrypoint for the benchmark VM

set -e

BENCHMARK_DIR="/benchmark"

BENCH_NAME=$1
SYSTEM=$2

print_help() {
    echo "Usage: $0 <benchmark_name> <system_type>"
    echo "  benchmark_name: The name of the benchmark to run."
    echo "  system_type: The type of system to run the benchmark on. 'linux' or 'asterinas'."
}

# Validate arguments
check_bench_name() {
    if [ -z "${BENCH_NAME}" ] || [ -z "${SYSTEM}" ]; then
        echo "Error: Invalid arguments."
        print_help
        exit 1
    fi

    local full_path="${BENCHMARK_DIR}/${BENCH_NAME}"

    if ! [ -d "${full_path}" ]; then
        echo "Directory '${BENCH_NAME}' does not exist in the benchmark directory."
        print_help
        exit 1
    fi
}

prepare_system() {
    if [ ! -d /tmp ]; then
        mkdir /tmp
    fi

    /sbin/ldconfig
    
    # System-specific preparation
    if [ "$SYSTEM" = "linux" ]; then
        mount -t devtmpfs devtmpfs /dev
        ip link set lo up
        modprobe virtio_blk
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
    check_bench_name

    # Prepare the system
    prepare_system

    # Run the benchmark
    BENCH_SCRIPT=${BENCHMARK_DIR}/${BENCH_NAME}/run.sh
    chmod +x ${BENCH_SCRIPT}
    ${BENCH_SCRIPT}

    # Shutdown explicitly if running on Linux
    if [ "$SYSTEM" = "linux" ]; then
        poweroff -f
    fi
}

main "$@"
