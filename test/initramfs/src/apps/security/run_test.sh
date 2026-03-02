#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

./capability/capabilities
./capability/capset
./capability/execve

./namespace/mnt_ns
./namespace/setns
./namespace/unshare
