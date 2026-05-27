#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

cd "$(dirname "$0")"

sh ./capability/run_test.sh

./lsm/module_selection
./lsm/yama
if [ -r /proc/self/attr/current ]; then
	./lsm/aster_mac/label_state
	./lsm/aster_mac/xattr_policy
fi

./namespace/cgroup_ns
./namespace/ipc_ns_sem
./namespace/mnt_ns
./namespace/proc_nsfs
./namespace/setns
./namespace/unshare
