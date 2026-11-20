#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -eu
case "$0" in
    */*) cd "${0%/*}" ;;
esac

for test in install verifier interpreter actions arguments stacking inheritance libseccomp libseccomp_rules libseccomp_runtime ptrace exit trap trace tsync exec_race strict proc_status log listener notification notification_lifecycle
do
    echo "=== seccomp: $test ==="
    "./$test"
done
