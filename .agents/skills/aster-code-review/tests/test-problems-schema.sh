#!/usr/bin/env bash
# Validate benchmark/problems.yaml against its schema (model-free).
# Run via `make -C tests test-problems-schema`.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
. "$HERE/lib.sh"
VALIDATE="$HERE/../benchmark/validate-problem-yaml.sh"

test_problems_yaml_passes_schema() {
    local out rc
    out="$("$VALIDATE" 2>&1)"; rc=$?
    assert_eq "validate-problem-yaml.sh exits 0"   "$rc"  0
    assert_contains "prints a summary" "$out" "problem_ids unique"
}

run_suite
