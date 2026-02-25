#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e

PROJECT_ROOT="$(realpath "$(dirname "${BASH_SOURCE[0]}")/..")"

CRATES=()
CARGO_OSDK_TEST_ARGS=()

usage() {
    cat <<EOF
Run kernel unit tests (ktest) for OSDK crates.

Usage:
  $(basename "$0") --crates <crate1,crate2,...> [-- CARGO_OSDK_TEST_ARGS...]

Options:
  --crates <crate1,crate2,...>   Comma-separated list of crate directories to test.
                                 (required)
  -h, --help                    Show this help message.

All arguments after '--' are forwarded to 'cargo osdk test'.

Examples:
  $(basename "$0") --crates "ostd,kernel" -- --target-arch=x86_64
EOF
}

parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            -h|--help)
                usage
                exit 0
                ;;
            --crates)
                IFS=',' read -ra CRATES <<< "$2"
                shift 2
                ;;
            --)
                shift
                CARGO_OSDK_TEST_ARGS=("$@")
                break
                ;;
            *)
                echo "Error: Unknown option '$1'" >&2
                echo
                usage
                exit 1
                ;;
        esac
    done

    if [[ ${#CRATES[@]} -eq 0 ]]; then
        echo "Error: --crates is required." >&2
        echo
        usage
        exit 1
    fi
}

run_ktest() {
    local failed=0

    for dir in "${CRATES[@]}"; do
        echo "[ktest] Testing $dir"

        if ! (cd "${PROJECT_ROOT}/${dir}" && cargo osdk test "${CARGO_OSDK_TEST_ARGS[@]}"); then
            echo "ERROR: Testing $dir failed"
            failed=1
        elif ! tail --lines 10 "${PROJECT_ROOT}/qemu.log" 2>/dev/null \
                | grep -q "^\[ktest runner\] All crates tested."; then
            echo "ERROR: Test verification failed for $dir"
            failed=1
        fi

        # Remove artifacts to save disk space (useful for CI runners).
        rm -rf "${PROJECT_ROOT}/target/osdk/"*
    done

    if [[ $failed -ne 0 ]]; then
        echo "SUMMARY: Some kernel-mode unit tests failed"
        exit 1
    else
        echo "SUMMARY: All kernel-mode unit tests passed"
    fi
}

parse_args "$@"
run_ktest