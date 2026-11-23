#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -eu

if [ "$#" -ne 1 ] || [ ! -d "$1" ]; then
    printf 'Usage: %s <directory containing the three upstream binaries>\n' "$0" >&2
    exit 2
fi

LC_ALL=C
export LC_ALL
log=$(mktemp)
trap 'rm -f "$log"' EXIT
trap 'exit 1' HUP INT TERM

failed=0
for test in instructions cycles ioctl_7_id; do
    printf 'Running upstream perf_event_tests/%s\n' "$test"
    if ! "$1/$test" > "$log" 2>&1 || ! grep -q ' PASSED$' "$log"; then
        cat "$log" >&2
        printf 'FAIL: upstream perf_event_tests/%s did not pass\n' "$test" >&2
        failed=1
        continue
    fi
    cat "$log"
done
if [ "$failed" -ne 0 ]; then
    exit 1
fi
printf 'PASS: upstream perf_event_tests instructions, cycles and ioctl_7_id\n'
