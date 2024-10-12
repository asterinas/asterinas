#!/bin/sh

# SPDX-License-Identifier: MPL-2.0
# Entrypoint for the benchmark VM

set -e

BENCHMARK_DIR="/benchmark"
READY_MESSAGE="The VM is ready for the benchmark."

BENCH_NAME=$1
SYSTEM="${2:-asterinas}"
echo "Running benchmark: ${BENCH_NAME} on ${SYSTEM}"

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

run_fio_test() {
    local mode="$1"
    local rw="$2"
    local size="$3"
    local bs="$4"

    local rwtype
    local name

    if [ "$mode" == "seq" ]; then
        name="seq-"
    elif [ "$mode" == "rnd" ]; then
        name="rnd-"
        rwtype="rand"
    else
        echo "Error: Invalid mode. Please use 'seq' or 'rnd'."
        exit 1
    fi

    if [ "$rw" == "r" ]; then
        name="${name}r-$bs"
        rwtype="${rwtype}read"
    elif [ "$rw" == "w" ]; then
        name="${name}w-$bs"
        rwtype="${rwtype}write"
    else
        echo "Error: Invalid rw. Please use 'r' or 'w'."
        exit 1
    fi
    
    /benchmark/bin/fio \
        --ioengine=sync \
        --size=$size \
        --rw=$rwtype \
        --filename=/dev/vda \
        --name=$name \
        --bs=$bs \
        --direct=1 \
        --numjobs=1 \
        --fsync_on_close=1 \
        --time_based=1 \
        --runtime=20 \
        --verify_fatal=1
}

prepare_system() {
    if [ ! -d /tmp ]; then
        mkdir /tmp
    fi

    /sbin/ldconfig
    
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
    check_bench_name

    # Prepare the system
    prepare_system
    
    # Message to notify the host script. It must align with the READY_MESSAGE in host_guest_bench_runner.sh.
    # DO NOT REMOVE THIS LINE!!!
    echo "${READY_MESSAGE}"

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
