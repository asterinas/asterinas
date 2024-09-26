#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e

BENCHMARK_PATH=$1
ASTERINAS_GUEST_CMD=$2
LINUX_GUEST_CMD=$3
ASTERINAS_OUTPUT=$4
LINUX_OUTPUT=$5

# Import the common functions
BENCHMARK_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" &>/dev/null && pwd)/../"
source "${BENCHMARK_DIR}/common/prepare_host.sh"

if [[ "$BENCHMARK_PATH" =~ "iperf" ]]; then 
    # Persist Iperf port
    export IPERF_PORT=5201
elif [[ "$BENCHMARK_PATH" =~ "nginx" ]]; then
    # Persist Nginx port
    export NGINX_PORT=8080
elif [[ "$BENCHMARK_PATH" =~ "redis" ]]; then
    # Persist Redis port
    export REDIS_PORT=6379
fi

# Function to run the benchmark
# Parameters:
#   $1: guest command to run on the VM
#   $2: output file to store the benchmark results
#   $3: sleep time before running the host command
run_benchmark() {
    local guest_cmd=$1
    local output_file=$2
    local sleep_time=$3

    echo "Running the benchmark on the VM..."
    eval "${guest_cmd}" & 
    sleep "${sleep_time}"  
    # Run the host command and save the output to the specified file.
    # You can also redirect the guest output to it.
    echo "Running the benchmark on the host..."
    bash "${BENCHMARK_PATH}/host.sh" | tee "${output_file}"  
}

# Run the benchmark on the Asterinas VM
# Use a sleep time of 2 minutes (2m) for the Asterinas VM
run_benchmark "${ASTERINAS_GUEST_CMD}" "${ASTERINAS_OUTPUT}" "2m"

# Run the benchmark on the Linux VM
# Use a sleep time of 20 seconds (20s) for the Linux VM
prepare_fs
run_benchmark "${LINUX_GUEST_CMD}" "${LINUX_OUTPUT}" "20s"