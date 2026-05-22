#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

cd "$(dirname "$0")"

sh ./capability/run_test.sh

./lsm/yama

./namespace/cgroup_ns
./namespace/ipc_ns_sem
./namespace/mnt_ns
./namespace/proc_nsfs
./namespace/setns
./namespace/unshare
