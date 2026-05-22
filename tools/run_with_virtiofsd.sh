#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -eu

if [ "$#" -eq 0 ]; then
    echo "Usage: $0 COMMAND [ARG]..." >&2
    exit 2
fi

VIRTIOFSD=${VIRTIOFSD:-/usr/libexec/virtiofsd}
VIRTIOFS_SOCKET=${VIRTIOFS_SOCKET:-/tmp/vhostqemu/vfs.sock}
VIRTIOFS_SCRATCH=${VIRTIOFS_SCRATCH:-off}
VIRTIOFS_SCRATCH_SOCKET=${VIRTIOFS_SCRATCH_SOCKET:-/tmp/vhostqemu/vfs-scratch.sock}
VIRTIOFS_SHARED_DIR=${VIRTIOFS_SHARED_DIR:-test/initramfs/build/virtiofs-test}
VIRTIOFS_SCRATCH_SHARED_DIR=${VIRTIOFS_SCRATCH_SHARED_DIR:-test/initramfs/build/virtiofs-scratch}

if [ ! -x "$VIRTIOFSD" ]; then
    echo "virtiofsd not found at $VIRTIOFSD. Set VIRTIOFSD=/path/to/virtiofsd." >&2
    exit 1
fi

virtiofsd_test_pid=
virtiofsd_scratch_pid=

cleanup_virtiofsd()
{
    kill "$virtiofsd_test_pid" "$virtiofsd_scratch_pid" 2>/dev/null || true
    wait "$virtiofsd_test_pid" "$virtiofsd_scratch_pid" 2>/dev/null || true
    rm -f "$VIRTIOFS_SOCKET" "$VIRTIOFS_SCRATCH_SOCKET"
}

start_virtiofsd()
{
    name=$1
    socket=$2
    shared_dir=$3

    mkdir -p "$shared_dir" "$(dirname "$socket")"
    find "$shared_dir" -mindepth 1 -maxdepth 1 -exec rm -rf {} +
    rm -f "$socket"

    "$VIRTIOFSD" \
        --shared-dir "$shared_dir" \
        --socket-path "$socket" \
        --sandbox none \
        --seccomp none \
        --cache auto \
        --xattr \
        > "virtiofsd-$name.log" 2>&1 &

    last_virtiofsd_pid=$!
    for _ in $(seq 1 100); do
        if [ -S "$socket" ]; then
            return 0
        fi
        if ! kill -0 "$last_virtiofsd_pid" 2>/dev/null; then
            echo "virtiofsd $name exited before creating $socket" >&2
            cat "virtiofsd-$name.log" >&2
            return 1
        fi
        sleep 0.1
    done

    echo "virtiofsd $name did not create $socket" >&2
    cat "virtiofsd-$name.log" >&2
    return 1
}

trap cleanup_virtiofsd EXIT INT TERM

start_virtiofsd test "$VIRTIOFS_SOCKET" "$VIRTIOFS_SHARED_DIR"
virtiofsd_test_pid=$last_virtiofsd_pid

if [ "$VIRTIOFS_SCRATCH" = "on" ]; then
    start_virtiofsd scratch "$VIRTIOFS_SCRATCH_SOCKET" "$VIRTIOFS_SCRATCH_SHARED_DIR"
    virtiofsd_scratch_pid=$last_virtiofsd_pid
fi

"$@"
