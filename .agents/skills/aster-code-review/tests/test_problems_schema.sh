#!/usr/bin/env bash

# SPDX-License-Identifier: MPL-2.0

# Validate benchmark/problems.yaml against its schema (model-free).
# Run via `make -C tests test_problems_schema`.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
. "$HERE/lib.sh"
VALIDATE="$HERE/../benchmark/validate_problem_yaml.sh"

test_problems_yaml_passes_schema() {
    local out rc
    out="$("$VALIDATE" 2>&1)"; rc=$?
    assert_eq "validate_problem_yaml.sh exits 0"   "$rc"  0
    assert_contains "prints a summary" "$out" "problem_ids unique"
}

run_suite
