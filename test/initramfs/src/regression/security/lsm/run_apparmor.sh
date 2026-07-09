#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

cd /test/security/lsm
./apparmor

echo "AppArmor regression tests passed."
