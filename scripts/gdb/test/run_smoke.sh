#!/bin/bash
# SPDX-License-Identifier: MPL-2.0
#
# Smoke test driver for Asterinas GDB debug helpers.
# Boots the kernel with a GDB server, runs printer assertions, and
# reports pass/fail.
#
# Usage:
#   ./scripts/gdb/test/run_smoke.sh
#   make gdb-smoke-test

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
SOCKET_PATH="$PROJECT_ROOT/.osdk-gdb-socket"

BUILD_TARGET_ARCH="${OSDK_TARGET_ARCH:-${TARGET_ARCH:-x86_64}}"
BUILD_PROFILE="dev"
QEMU_PID=""
SMOKE_LOG=""
GDB_STATUS=0

die() {
    echo "Error: $1" >&2
    exit 1
}

ensure_rust_gdb() {
    if ! command -v rust-gdb >/dev/null 2>&1; then
        die "rust-gdb is required. Use rustup or the Asterinas dev container."
    fi
}

target_triple() {
    case "$1" in
        aarch64)
            echo "aarch64-unknown-none-softfloat"
            ;;
        riscv64)
            echo "riscv64imac-unknown-none-elf"
            ;;
        x86_64)
            echo "x86_64-unknown-none"
            ;;
        loongarch64)
            echo "loongarch64-unknown-none-softfloat"
            ;;
        *)
            echo "Error: unsupported target architecture '$1'" >&2
            return 1
            ;;
    esac
}

artifact_profile() {
    case "$1" in
        dev)
            echo "debug"
            ;;
        *)
            echo "$1"
            ;;
    esac
}

parse_build_args() {
    local pending_arg=""

    for arg in "$@"; do
        if [ "$pending_arg" = "profile" ]; then
            BUILD_PROFILE="$arg"
            pending_arg=""
            continue
        fi
        if [ "$pending_arg" = "target_arch" ]; then
            BUILD_TARGET_ARCH="$arg"
            pending_arg=""
            continue
        fi

        case "$arg" in
            --profile)
                pending_arg="profile"
                ;;
            --profile=*)
                BUILD_PROFILE="${arg#--profile=}"
                ;;
            --release)
                BUILD_PROFILE="release"
                ;;
            --target-arch)
                pending_arg="target_arch"
                ;;
            --target-arch=*)
                BUILD_TARGET_ARCH="${arg#--target-arch=}"
                ;;
        esac
    done

    if [ -n "$pending_arg" ]; then
        die "missing value after --${pending_arg/_/-}"
    fi
}

# shellcheck disable=SC2317 # Invoked by the EXIT trap.
cleanup() {
    if [ -n "$QEMU_PID" ]; then
        kill -- -"$QEMU_PID" 2>/dev/null || kill "$QEMU_PID" 2>/dev/null || true
        wait "$QEMU_PID" 2>/dev/null || true
    fi
    rm -f "$SOCKET_PATH"
    if [ -n "$SMOKE_LOG" ]; then
        rm -f "$SMOKE_LOG"
    fi
}

reset_gdb_socket() {
    # `cargo osdk run` creates this socket beside OSDK.toml.
    rm -f "$SOCKET_PATH"
}

start_kernel_gdb_server() {
    echo "[smoke] Starting kernel with GDB server..."
    (
        cd "$PROJECT_ROOT"
        setsid cargo osdk run "$@" --gdb-server wait-client
    ) &
    QEMU_PID=$!
}

wait_for_gdb_socket() {
    local timeout_secs=120

    echo "[smoke] Waiting for GDB socket..."
    for _ in $(seq 1 "$timeout_secs"); do
        [ -S "$SOCKET_PATH" ] && return
        sleep 1
    done

    die "GDB socket did not appear after $timeout_secs seconds"
}

kernel_elf() {
    local target
    local profile_dir
    local elf

    target="$(target_triple "$BUILD_TARGET_ARCH")"
    profile_dir="$(artifact_profile "$BUILD_PROFILE")"
    elf="$PROJECT_ROOT/target/$target/$profile_dir/aster-kernel-osdk-bin"

    if [ ! -f "$elf" ]; then
        die "kernel ELF with debug symbols not found: $elf"
    fi
    echo "$elf"
}

run_gdb_assertions() {
    local elf="$1"

    echo "[smoke] Kernel ELF: $elf"
    echo "[smoke] Running GDB assertions..."
    SMOKE_LOG="$(mktemp)"

    set +e
    (
        cd "$PROJECT_ROOT"
        timeout 120 rust-gdb --batch \
            --command=scripts/gdb/test/smoke.gdb \
            "$elf"
    ) 2>&1 | tee "$SMOKE_LOG"
    GDB_STATUS=${PIPESTATUS[0]}
    set -e
}

report_result() {
    if [ "$GDB_STATUS" -ne 0 ]; then
        echo "[smoke] FAILED - rust-gdb exited with status $GDB_STATUS"
        exit 1
    fi

    if grep -q "^SMOKE: all ok$" "$SMOKE_LOG"; then
        echo "[smoke] PASSED"
        exit 0
    fi

    echo "[smoke] FAILED - see output above"
    exit 1
}

main() {
    local elf

    parse_build_args "$@"
    ensure_rust_gdb
    reset_gdb_socket
    start_kernel_gdb_server "$@"
    wait_for_gdb_socket
    elf="$(kernel_elf)"
    run_gdb_assertions "$elf"
    report_result
}

trap cleanup EXIT
main "$@"
