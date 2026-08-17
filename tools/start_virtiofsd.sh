#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -eu

VIRTIOFSD=${VIRTIOFSD:-/usr/libexec/virtiofsd}
VIRTIOFS_SOCKET=${VIRTIOFS_SOCKET:-/tmp/vhostqemu/vfs.sock}
VIRTIOFS_SHARED_DIR=${VIRTIOFS_SHARED_DIR:-test/initramfs/build/virtiofs-test}
VIRTIOFS_SCRATCH=${VIRTIOFS_SCRATCH:-off}
VIRTIOFS_SCRATCH_SOCKET=${VIRTIOFS_SCRATCH_SOCKET:-/tmp/vhostqemu/vfs-scratch.sock}
VIRTIOFS_SCRATCH_SHARED_DIR=${VIRTIOFS_SCRATCH_SHARED_DIR:-test/initramfs/build/virtiofs-scratch}
VIRTIOFS_LOG=${VIRTIOFS_LOG:-virtiofsd.log}

if [ ! -x "$VIRTIOFSD" ]; then
    echo "virtiofsd not found at $VIRTIOFSD; set VIRTIOFSD=/path/to/virtiofsd" >&2
    exit 1
fi

start_virtiofsd()
{
    socket=$1
    shared_dir=$2
    log=$3
    VIRTIOFSD_PID=''

    mkdir -p "$shared_dir" "$(dirname "$socket")"
    if ! : > "$log"; then
        echo "cannot write virtiofsd log $log" >&2
        return 1
    fi

    rm -f "$socket"
    "$VIRTIOFSD" \
        --shared-dir "$shared_dir" \
        --socket-path "$socket" \
        --cache auto \
        --xattr \
        > "$log" 2>&1 &

    pid=$!
    for _ in $(seq 1 100); do
        if [ -S "$socket" ]; then
            if kill -0 "$pid" 2>/dev/null; then
                VIRTIOFSD_PID=$pid
                return 0
            fi
            echo "virtiofsd exited after creating $socket" >&2
            cat "$log" >&2 || true
            rm -f "$socket"
            return 1
        fi
        if ! kill -0 "$pid" 2>/dev/null; then
            echo "virtiofsd exited before creating $socket" >&2
            cat "$log" >&2 || true
            rm -f "$socket"
            return 1
        fi
        sleep 0.1
    done

    echo "virtiofsd did not create $socket" >&2
    cat "$log" >&2 || true
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
    rm -f "$socket"
    return 1
}

start_virtiofsd "$VIRTIOFS_SOCKET" "$VIRTIOFS_SHARED_DIR" "$VIRTIOFS_LOG"
primary_pid=$VIRTIOFSD_PID
if [ "$VIRTIOFS_SCRATCH" = "on" ]; then
    if ! start_virtiofsd "$VIRTIOFS_SCRATCH_SOCKET" "$VIRTIOFS_SCRATCH_SHARED_DIR" \
        "${VIRTIOFS_LOG%.log}-scratch.log"; then
        kill "$primary_pid" 2>/dev/null || true
        wait "$primary_pid" 2>/dev/null || true
        rm -f "$VIRTIOFS_SOCKET"
        exit 1
    fi
fi
