#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

KVMTEST_DIR=$(dirname "$0")

if [ ! -c /dev/kvm ]; then
    echo "[ SKIPPED ] kvm-hello-world: /dev/kvm is unavailable"
    exit 0
fi

# Exercise all four modes supported by kvm-hello-world.
for mode in -r -s -p -l; do
    "$KVMTEST_DIR/kvm-hello-world" "$mode"
done
