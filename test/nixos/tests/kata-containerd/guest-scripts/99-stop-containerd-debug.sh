#!/bin/sh
set -u

OUT_DIR=${KATA_DEBUG_OUT:-/tmp/kata-debug-out}

if [ -r "$OUT_DIR/qemu-serial-watcher.pid" ]; then
  kill "$(cat "$OUT_DIR/qemu-serial-watcher.pid")" 2>/dev/null || true
fi

if [ -r "$OUT_DIR/containerd.pid" ]; then
  kill "$(cat "$OUT_DIR/containerd.pid")" 2>/dev/null || true
fi

rm -f /tmp/containerd-debug.sock
systemctl start containerd.socket containerd 2>/dev/null || true
echo "stopped debug containerd and tried to restore systemd containerd"
