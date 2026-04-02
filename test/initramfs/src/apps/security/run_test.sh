#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

./aster/xattr_policy

./capability/capabilities
./capability/capset
./capability/execve

./namespace/mnt_ns
./namespace/proc_nsfs
./namespace/setns
./namespace/unshare

./yama/pidfd_getfd_scope
