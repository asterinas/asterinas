#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -eu

results=$(mktemp)
trap 'rm -f "$results"' EXIT
/benchmark/bin/seccomp-bench > "$results"
cat "$results"
awk -F, '$1 == 1 && $2 == 64 && $3 == 1 { total += $5; count++ }
    END {
        if (count != 5) exit 1;
        printf "Seccomp syscall ns: %.3f\n", total / count;
    }' "$results"
