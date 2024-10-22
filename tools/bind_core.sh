#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

# This script binds all vCPU threads of a running QEMU process 
# to CPUs within a single NUMA node.
# The QEMU process must be monitored via `SOCK_MONITOR` 
# to retrieve each vCPU's thread ID.

# Usage: ./bind_core.sh <SOCK_MONITOR>

set -e

# Execute a command through the monitor socket and return the result.
# Usage: exec_qemu_monitor_cmd <sock_monitor> <cmd>
exec_qemu_monitor_cmd() {
    local client=$1
    local cmd=$2
    printf "%s\n" "${cmd}" | socat unix-client:"${client}" stdio | tail -n +2 | grep -v "^(qemu)" | tr -d '\r'
    return $?
}

# Retrieve thread IDs of all vCPUs.
# Usage: get_vcpu_tids <sock_monitor>
get_vcpu_tids() {
    res=$(exec_qemu_monitor_cmd "$1" "info cpus")
    echo $res | grep -oP 'thread_id=\K\d+'
}

# Get 'N' CPUs in the same NUMA node as CPU 0.
# Usage: get_cpus_in_numa_node <N>
get_cpus_in_numa_node() {
    local required_cpus=$1
    local total_cpus=$(lscpu | awk '/^CPU\(s\):/ {print $2}')
    local cpu0_numa_id=$(cat "/sys/devices/system/cpu/cpu0/topology/physical_package_id")
    local cpu_list=()
    local count=0

    for cpu in $(seq 0 $((total_cpus - 1))); do
        local cpu_numa_id=$(cat "/sys/devices/system/cpu/cpu${cpu}/topology/physical_package_id" 2>/dev/null || echo "")
        
        if [ "$cpu_numa_id" = "$cpu0_numa_id" ]; then
            cpu_list+=($cpu)
            count=$((count + 1))
        fi
        
        if [ "$count" -ge "$required_cpus" ]; then
            break
        fi
    done
    echo "${cpu_list[@]}"
}

SOCK_MONITOR=$1
vcpu_tids=$(get_vcpu_tids "${SOCK_MONITOR}")
vcpu_num=$(echo "${vcpu_tids}" | wc -w)
cpus=$(get_cpus_in_numa_node "${vcpu_num}")

# Bind vCPUs
cpus_array=(${cpus})
n=0
for vcpu_tid in ${vcpu_tids}; do
    cpu=${cpus_array[$n]}
    n=$((n + 1))
    taskset -pc ${cpu} ${vcpu_tid}
done
