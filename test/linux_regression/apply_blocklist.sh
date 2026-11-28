#!/usr/bin/env bash

# SPDX-License-Identifier: MPL-2.0

set -euo pipefail

usage() {
    cat <<'HELP'
Usage: ./apply_blocklist.sh BLOCKLIST TEST_DIR

Replace each blocklisted regression test with a script that prints SKIP and
exits successfully. Called by default.nix at build time.

Arguments:
  BLOCKLIST  File listing one test path per line (relative to TEST_DIR)
  TEST_DIR   Built regression test binary directory (modified in place)

Example:
  ./apply_blocklist.sh blocklist /tmp/regression-tests
HELP
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
    usage
    exit 0
fi
if (( $# != 2 )); then
    usage >&2
    exit 2
fi

blocklist=$1
test_dir=$2

if [[ ! -f "$blocklist" ]]; then
    echo "apply_blocklist.sh: blocklist not found: $blocklist" >&2
    exit 1
fi
if [[ ! -d "$test_dir" ]]; then
    echo "apply_blocklist.sh: not a directory: $test_dir" >&2
    exit 1
fi

test_root=$(realpath -- "$test_dir")
declare -A seen_paths=()
test_paths=()
errors=0
lineno=0

# Validate the whole list before replacing any test.
mapfile -t lines < "$blocklist"
for test_path in "${lines[@]}"; do
    lineno=$((lineno + 1))
    if [[ -z "$test_path" || "$test_path" == \#* ]]; then
        continue
    fi
    if [[ "$test_path" == /* || ! "$test_path" =~ ^[A-Za-z0-9_./-]+$ ]]; then
        echo "$blocklist:$lineno: must be a relative path of [A-Za-z0-9_./-]: $test_path" >&2
        errors=1
        continue
    fi

    if ! resolved_path=$(realpath -e -- "$test_root/$test_path" 2>/dev/null); then
        echo "$blocklist:$lineno: missing test: $test_path" >&2
        errors=1
        continue
    fi

    filename=$(basename -- "$resolved_path")
    if [[ "$resolved_path" != "$test_root/"* ]]; then
        echo "$blocklist:$lineno: path escapes TEST_DIR: $test_path" >&2
        errors=1
    elif [[ "$filename" == "run_test.sh" || "$filename" == "run_regression_test.sh" ]]; then
        echo "$blocklist:$lineno: cannot blocklist runner script: $test_path" >&2
        errors=1
    elif [[ -n "${seen_paths[$resolved_path]:-}" ]]; then
        echo "$blocklist:$lineno: duplicate entry: $test_path" >&2
        errors=1
    elif [[ ! -f "$resolved_path" || ! -x "$resolved_path" ]]; then
        echo "$blocklist:$lineno: not an executable file: $test_path" >&2
        errors=1
    else
        seen_paths[$resolved_path]=1
        test_paths+=("$test_path")
    fi
done

if (( errors != 0 )); then
    exit 1
fi

# Replace each blocklisted test with an executable skip stub.
for test_path in "${test_paths[@]}"; do
    stub_path="$test_root/$test_path"
    rm -f -- "$stub_path"
    cat <<EOF > "$stub_path"
#!/usr/bin/env sh
echo "SKIP $test_path"
exit 0
EOF
    chmod 0755 "$stub_path"
done
