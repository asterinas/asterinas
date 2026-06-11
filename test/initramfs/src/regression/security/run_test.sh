#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

./capability/capabilities
./capability/capset
./capability/execve
./capability/kill
./capability/reboot
./capability/setgroups
./capability/trusted_xattr

./lsm/module_selection
./lsm/yama

./namespace/cgroup_ns
./namespace/ipc_ns_sem
./namespace/mnt_ns
./namespace/proc_nsfs
./namespace/setns
./namespace/unshare
