#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e

BENCHMARK_PATH=$1
ASTERINAS_GUEST_CMD=$2
LINUX_GUEST_CMD=$3
ASTERINAS_OUTPUT=$4
LINUX_OUTPUT=$5
# Message to monitor in the log file to determine whether the VM is ready
# It should align with bench_runner.sh
READY_MESSAGE="The VM is ready for the benchmark."

# Import the common functions
BENCHMARK_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" &>/dev/null && pwd)/../"
source "${BENCHMARK_ROOT}/common/prepare_host.sh"

if [[ "$BENCHMARK_PATH" =~ "iperf" ]]; then 
    # Persist Iperf port
    export IPERF_PORT=5201
elif [[ "$BENCHMARK_PATH" =~ "nginx" ]]; then
    # Persist Nginx port
    export NGINX_PORT=8080
elif [[ "$BENCHMARK_PATH" =~ "redis" ]]; then
    # Persist Redis port
    export REDIS_PORT=6379
elif [[ "$BENCHMARK_PATH" =~ "tcp_virtio_lat" ]]; then
    # Persist lmbench/tcp_lat port
    export LMBENCH_TCP_LAT_PORT=31234
elif [[ "$BENCHMARK_PATH" =~ "tcp_virtio_bw" ]]; then
    # Persist lmbench/bw_tcp port
    export LMBENCH_TCP_BW_PORT=31236
elif [[ "$BENCHMARK_PATH" =~ "memcached" ]]; then
    # Persist Memcached port
    export MEMCACHED_PORT=11211
fi

# Function to run the benchmark
# Parameters:
#   $1: guest command to run on the VM
#   $2: output file to store the benchmark results
#   $3: log file to monitor for the ready message
#   $4: ready message to monitor in the log file
run_benchmark() {
    local guest_cmd=$1
    local output_file=$2
    local guest_log_file=$3
    local ready_message=$4

    echo "Running the benchmark on the VM..."
    eval "${guest_cmd}" | tee "${guest_log_file}" & 

    # Monitor the log file for the ready message
    echo "Waiting for the ready message: ${ready_message}"
    while true; do
        if grep -q "${ready_message}" "${guest_log_file}"; then
            echo "Ready message detected. Running the benchmark on the host..."
            break
        fi
        sleep 1
    done

    # Sleep for a short time to ensure the guest is fully ready
    sleep 1

    # The listening server address in guest VM
    export GUEST_SERVER_IP_ADDRESS=10.0.2.15

    # Run the host command and save the output to the specified file.
    bash "${BENCHMARK_PATH}/host.sh" 2>&1 | tee "${output_file}"  

    # Clean up the log file
    rm -f "${guest_log_file}"
}

# Run the benchmark on the Asterinas VM
run_benchmark "${ASTERINAS_GUEST_CMD}" "${ASTERINAS_OUTPUT}" "/tmp/asterinas.log" "${READY_MESSAGE}" 

# Wait for the Asterinas QEMU process to exit
wait

# Run the benchmark on the Linux VM
prepare_fs
run_benchmark "${LINUX_GUEST_CMD}" "${LINUX_OUTPUT}" "/tmp/linux.log" "${READY_MESSAGE}"

# Wait for the Linux QEMU process to exit
wait