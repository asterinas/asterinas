#!/bin/sh
set -u

OUT_DIR=${KATA_DEBUG_OUT:-/tmp/kata-debug-out}
ADDR=${CONTAINERD_ADDRESS:-/tmp/containerd-debug.sock}
CID=${KATA_PROBE_ID:-kata-rootfs-probe}
ROOTFS=${KATA_ROOTFS:-/tmp/kata-rootfs}

mkdir -p "$OUT_DIR"

if [ ! -S "$ADDR" ]; then
  if [ -S /run/containerd/containerd.sock ]; then
    ADDR=/run/containerd/containerd.sock
  else
    echo "no containerd socket found; run kata-debug-start-containerd first" >&2
    exit 1
  fi
fi

echo "containerd address: $ADDR" | tee "$OUT_DIR/probe.log"
echo "container id: $CID" | tee -a "$OUT_DIR/probe.log"

ctr --address "$ADDR" tasks kill "$CID" >/dev/null 2>&1 || true
ctr --address "$ADDR" containers rm "$CID" >/dev/null 2>&1 || true
rm -rf "$ROOTFS" /tmp/kata-run.txt \
  "/tmp/containerd-state/io.containerd.runtime.v2.task/default/$CID" \
  "/run/kata/$CID"

mkdir -p "$ROOTFS/bin"

SH=$(readlink -f /run/current-system/sw/bin/sh)
mkdir -p "$ROOTFS$(dirname "$SH")"
cp "$SH" "$ROOTFS$SH"
cp "$SH" "$ROOTFS/bin/sh"

ldd "$SH" \
  | sed -n 's/.*=> \(\/nix\/store\/[^ ]*\).*/\1/p; s/^[[:space:]]*\(\/nix\/store\/[^ ]*\).*/\1/p' \
  | while read -r dep; do
      mkdir -p "$ROOTFS$(dirname "$dep")"
      cp "$dep" "$ROOTFS$dep"
    done

# PT_INTERP detection (no readelf on minimal NixOS) + lib/lib64 mirror.
INTERP=$(LC_ALL=C tr '\0' '\n' < "$SH" 2>/dev/null | grep -m1 '/ld-linux' || true)
echo "PT_INTERP_DETECTED=$INTERP" | tee -a "$OUT_DIR/probe.log"
if [ -n "$INTERP" ]; then
  mkdir -p "$ROOTFS$(dirname "$INTERP")"
  if [ -e "$INTERP" ] && [ ! -e "$ROOTFS$INTERP" ]; then
    cp "$INTERP" "$ROOTFS$INTERP"
  fi
fi
for d in "$ROOTFS"/nix/store/*-glibc-*/; do
  [ -d "$d" ] || continue
  mkdir -p "$d"lib "$d"lib64
  for f in "$d"lib64/ld-linux* "$d"lib/ld-linux*; do
    [ -f "$f" ] || continue
    base=$(basename "$f")
    [ -e "$d"lib/"$base" ]   || cp "$f" "$d"lib/"$base"   2>/dev/null || true
    [ -e "$d"lib64/"$base" ] || cp "$f" "$d"lib64/"$base" 2>/dev/null || true
  done
done

echo "rootfs files:" | tee -a "$OUT_DIR/probe.log"
find "$ROOTFS" -type f | sed -n '1,40p' | tee -a "$OUT_DIR/probe.log"
echo "rootfs ld-linux:" | tee -a "$OUT_DIR/probe.log"
find "$ROOTFS" -name 'ld-linux*' -ls | tee -a "$OUT_DIR/probe.log"

(
  i=0
  while :; do
    {
      echo "=== live $i $(date) ==="
      echo "--- rootfs ---"
      find "$ROOTFS" -maxdepth 6 -ls 2>&1 || true
      echo "--- containerd task state ---"
      find "/tmp/containerd-state/io.containerd.runtime.v2.task/default/$CID" \
        -maxdepth 8 -ls 2>&1 || true
      echo "--- kata state ---"
      find /run/kata /run/kata-containers -maxdepth 10 -ls 2>&1 || true
      echo "--- kata/containerd mounts ---"
      mount 2>&1 | grep -Ei 'kata|containerd|virtio|tmp/kata' || true
    } >> "$OUT_DIR/rootfs-live-state.log" 2>&1
    i=$((i + 1))
    sleep 1
  done
) &
SNAP_PID=$!

timeout 120s ctr --debug \
  --address "$ADDR" \
  run --rootfs \
  --runtime io.containerd.kata.v2 \
  "$ROOTFS" \
  "$CID" \
  /bin/sh -c 'echo ok-from-kata' \
  > /tmp/kata-run.txt 2>&1
STATUS=$?

kill "$SNAP_PID" >/dev/null 2>&1 || true
wait "$SNAP_PID" 2>/dev/null || true

echo "exit:$STATUS" >> /tmp/kata-run.txt
cat /tmp/kata-run.txt | tee -a "$OUT_DIR/probe.log"

{
  echo "=== rootfs tree ==="
  find "$ROOTFS" -maxdepth 6 -ls 2>&1 || true

  echo
  echo "=== containerd task state ==="
  find "/tmp/containerd-state/io.containerd.runtime.v2.task/default/$CID" \
    -maxdepth 6 -ls 2>&1 || true

  echo
  echo "=== oci config ==="
  cat "/tmp/containerd-state/io.containerd.runtime.v2.task/default/$CID/config.json" \
    2>&1 || true

  echo
  echo "=== kata state ==="
  find /run/kata /run/kata-containers -maxdepth 8 -ls 2>&1 || true

  echo
  echo "=== kata/containerd mounts ==="
  mount 2>&1 | grep -Ei 'kata|containerd|virtio|tmp/kata' || true
} > "$OUT_DIR/rootfs-state.log" 2>&1

exit 0
