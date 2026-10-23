#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -eu
case "$0" in
    */*) cd "${0%/*}" ;;
esac

for test in install verifier interpreter actions arguments stacking inheritance libseccomp ptrace exit trap
do
    echo "=== seccomp: $test ==="
    "./$test"
done
