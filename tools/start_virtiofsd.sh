#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -eu

VIRTIOFSD=${VIRTIOFSD:-/usr/libexec/virtiofsd}
VIRTIOFS_SOCKET=${VIRTIOFS_SOCKET:-/tmp/vhostqemu/vfs.sock}
VIRTIOFS_SHARED_DIR=${VIRTIOFS_SHARED_DIR:-test/initramfs/build/virtiofs}
VIRTIOFS_LOG=${VIRTIOFS_LOG:-virtiofsd.log}

if [ ! -x "$VIRTIOFSD" ]; then
    echo "virtiofsd not found at $VIRTIOFSD. Set VIRTIOFSD=/path/to/virtiofsd." >&2
    exit 1
fi

mkdir -p "$VIRTIOFS_SHARED_DIR" "$(dirname "$VIRTIOFS_SOCKET")"
if [ -f "${VIRTIOFS_SOCKET}.pid" ]; then
    old_pid=$(cat "${VIRTIOFS_SOCKET}.pid")
    old_comm=
    if [ -n "$old_pid" ] && [ -r "/proc/$old_pid/comm" ]; then
        old_comm=$(cat "/proc/$old_pid/comm")
    fi
    if [ "$old_comm" = "virtiofsd" ]; then
        kill "$old_pid" 2>/dev/null || true
        wait "$old_pid" 2>/dev/null || true
    fi
    rm -f "${VIRTIOFS_SOCKET}.pid"
fi
rm -f "$VIRTIOFS_SOCKET"

"$VIRTIOFSD" \
    --shared-dir "$VIRTIOFS_SHARED_DIR" \
    --socket-path "$VIRTIOFS_SOCKET" \
    --sandbox none \
    --seccomp none \
    --cache auto \
    --xattr \
    > "$VIRTIOFS_LOG" 2>&1 &

virtiofsd_pid=$!

for _ in $(seq 1 100); do
    if [ -S "$VIRTIOFS_SOCKET" ]; then
        echo "$virtiofsd_pid" > "${VIRTIOFS_SOCKET}.pid"
        exit 0
    fi
    if ! kill -0 "$virtiofsd_pid" 2>/dev/null; then
        echo "virtiofsd exited before creating $VIRTIOFS_SOCKET" >&2
        cat "$VIRTIOFS_LOG" >&2
        exit 1
    fi
    sleep 0.1
done

echo "virtiofsd did not create $VIRTIOFS_SOCKET" >&2
cat "$VIRTIOFS_LOG" >&2
kill "$virtiofsd_pid" 2>/dev/null || true
wait "$virtiofsd_pid" 2>/dev/null || true
exit 1
