#!/bin/sh
set -u

OUT_DIR=${KATA_DEBUG_OUT:-/tmp/kata-debug-out}
ADDR=${CONTAINERD_ADDRESS:-/tmp/containerd-debug.sock}
ROOT=${CONTAINERD_ROOT:-/tmp/containerd-root}
STATE=${CONTAINERD_STATE:-/tmp/containerd-state}

mkdir -p "$OUT_DIR"

find_containerd_config() {
  PID=$(pidof containerd 2>/dev/null | awk '{print $1}')
  if [ -n "${PID:-}" ]; then
    tr '\0' '\n' < "/proc/$PID/cmdline" 2>/dev/null \
      | grep -m1 'containerd-config-checked.toml' && return 0
  fi

  find /etc /run/current-system /nix/store \
    -name 'containerd-config-checked.toml' -type f 2>/dev/null \
    | head -n1
}

CONFIG=$(find_containerd_config || true)
if [ -z "${CONFIG:-}" ] || [ ! -r "$CONFIG" ]; then
  echo "failed to find readable containerd-config-checked.toml" >&2
  exit 1
fi

echo "containerd config: $CONFIG" | tee "$OUT_DIR/containerd-start.log"

systemctl stop containerd.socket containerd 2>&1 | tee -a "$OUT_DIR/containerd-start.log" || true

rm -f "$ADDR"
mkdir -p "$ROOT" "$STATE"

KATA_CONF_FILE=/etc/kata-containers/configuration.toml \
containerd \
  --log-level debug \
  --address "$ADDR" \
  --root "$ROOT" \
  --state "$STATE" \
  --config "$CONFIG" \
  > "$OUT_DIR/containerd.log" 2>&1 &

PID=$!
echo "$PID" > "$OUT_DIR/containerd.pid"
echo "debug containerd pid: $PID" | tee -a "$OUT_DIR/containerd-start.log"
echo "debug containerd address: $ADDR" | tee -a "$OUT_DIR/containerd-start.log"

for _ in $(seq 1 50); do
  if [ -S "$ADDR" ]; then
    echo "debug containerd socket ready: $ADDR" | tee -a "$OUT_DIR/containerd-start.log"
    exit 0
  fi
  sleep 0.2
done

echo "debug containerd socket not ready" | tee -a "$OUT_DIR/containerd-start.log"
tail -80 "$OUT_DIR/containerd.log" 2>/dev/null || true
exit 1
