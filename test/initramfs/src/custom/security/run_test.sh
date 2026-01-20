#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

./capability/capabilities

./namespace/mnt_ns
./namespace/setns
./namespace/unshare
