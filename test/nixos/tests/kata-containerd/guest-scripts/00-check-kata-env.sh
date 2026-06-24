#!/bin/sh
set -u

OUT_DIR=${KATA_DEBUG_OUT:-/tmp/kata-debug-out}
mkdir -p "$OUT_DIR"

echo "=== containerd ==="
systemctl is-active containerd 2>&1 || true
ls -l /run/containerd/containerd.sock 2>&1 || true
ctr version 2>&1 || true

echo
echo "=== kata runtime-rs ==="
command -v containerd-shim-kata-v2 2>&1 || true
containerd-shim-kata-v2 --version 2>&1 || true
test -r /etc/kata-containers/configuration.toml && echo kata-rs-config-ready || echo kata-rs-config-missing

echo
echo "=== containerd kata config ==="
PID=$(pidof containerd 2>/dev/null | awk '{print $1}')
if [ -n "${PID:-}" ]; then
  tr '\0' '\n' < "/proc/$PID/cmdline" 2>/dev/null || true
  CONFIG=$(tr '\0' '\n' < "/proc/$PID/cmdline" 2>/dev/null | grep -m1 'containerd-config-checked.toml' || true)
  echo "CONFIG=${CONFIG:-}"
  if [ -n "${CONFIG:-}" ]; then
    grep -n 'io.containerd.kata.v2' "$CONFIG" 2>&1 || true
  fi
else
  echo "containerd pid not found"
fi

echo
echo "=== vhost-vsock ==="
ls -l /dev/vhost-vsock 2>&1 || true

echo
echo "=== output dir ==="
echo "$OUT_DIR"
