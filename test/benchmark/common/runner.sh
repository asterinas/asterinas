#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

print_help() {
    echo "Usage: $0 bench_name"
    echo ""
    echo "The bench_name argument must be one of the directory under asterinas/test/benchmark/".
}

BENCH_NAME=$1
SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)

# Validate arguments
check_bench_name() {
    if [ -z "${BENCH_NAME}" ]; then
        echo "Error: No directory provided."
        print_help
        exit 1
    fi

    local full_path="${SCRIPT_DIR}/../${BENCH_NAME}"

    if ! [ -d "${full_path}" ]; then
        echo "Directory '${BENCH_NAME}' does not exist in the script directory."
        print_help
        exit 1
    fi
}

check_bench_name

BENCH_SCRIPT=${SCRIPT_DIR}/../${BENCH_NAME}/run.sh

# Prepare the environment
if [ ! -d /tmp ]; then
    mkdir /tmp
fi
/sbin/ldconfig
chmod +x ${BENCH_SCRIPT}

# Run the benchmark
${BENCH_SCRIPT}