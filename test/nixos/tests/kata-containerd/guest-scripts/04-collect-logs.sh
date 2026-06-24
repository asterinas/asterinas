#!/bin/sh
set -u

OUT_DIR=${KATA_DEBUG_OUT:-/tmp/kata-debug-out}
mkdir -p "$OUT_DIR"

JOURNAL_LOG="$OUT_DIR/journal.log"
CONTAINERD_LOG="$OUT_DIR/containerd.log"
INTERESTING_LOG="$OUT_DIR/containerd-interesting.log"
PROCESS_LOG="$OUT_DIR/processes.log"
SOCKET_LOG="$OUT_DIR/sockets.log"
SUMMARY_LOG="$OUT_DIR/summary.log"

{
  echo "=== journald status ==="
  systemctl status systemd-journald.service systemd-journald.socket systemd-journald-dev-log.socket --no-pager -l 2>&1 || true
  echo
  echo "=== failed units ==="
  systemctl --failed --no-pager 2>&1 || true
  echo
  echo "=== journal tail ==="
  journalctl -b --no-pager -n 240 2>&1 || true
} > "$JOURNAL_LOG"

if [ -r "$CONTAINERD_LOG" ]; then
  grep -Ei 'QemuInner|qemu stderr|qemu console|vm console|vhost|VHOST|vsock|SetRunning|SET_RUNNING|EOPNOTSUPP|Operation not supported|not implemented|virtiofsd|Client connected|Client disconnected|Starting QEMU VM|qemu process started|QMP|console\.sock|Connection refused|Protocol not available|failed|error|timeout|agent|exit' \
    "$CONTAINERD_LOG" > "$INTERESTING_LOG" 2>&1 || true
else
  echo "containerd log missing: $CONTAINERD_LOG" > "$INTERESTING_LOG"
fi

for log in "$OUT_DIR"/qemu-wrapper-*.stderr; do
  [ -e "$log" ] || continue
  grep -Ei 'vhost|vsock|Operation not supported|not supported|failed|error|qemu|kvm|virtio' \
    "$log" >> "$INTERESTING_LOG" 2>&1 || true
done

{
  echo "=== kata/containerd/qemu processes ==="
  ps -ef 2>&1 | grep -Ei 'containerd|kata|qemu|virtiofsd' | grep -v grep || true

  echo
  echo "=== qemu fd state ==="
  for pid in $(pidof qemu-system-x86_64 2>/dev/null || true); do
    echo "--- qemu pid $pid ---"
    tr '\0' ' ' < "/proc/$pid/cmdline" 2>/dev/null || true
    echo
    ls -l "/proc/$pid/fd" 2>&1 || true
  done
} > "$PROCESS_LOG"

{
  echo "=== runtime state paths ==="
  find /tmp/containerd-state /run/kata /run/containerd -maxdepth 8 2>/dev/null \
    | grep -Ei 'kata-rootfs-probe|console\.sock|qmp|virtiofsd|vsock|sandbox' || true

  echo
  echo "=== socket files ==="
  find /tmp/containerd-state /run/kata /run/containerd -name '*.sock' -type s \
    -exec ls -l {} \; 2>/dev/null || true
} > "$SOCKET_LOG"

{
  echo "=== kata rootfs run exit ==="
  tail -40 /tmp/kata-run.txt 2>/dev/null || true

  echo
  echo "=== kata image run exit ==="
  tail -80 /tmp/kata-image-run.txt 2>/dev/null || true

  echo
  echo "=== image probe log ==="
  cat "$OUT_DIR/image-probe.log" 2>/dev/null || true

  echo
  echo "=== qemu wrapper stderr ==="
  for log in "$OUT_DIR"/qemu-wrapper-*.stderr; do
    [ -e "$log" ] || continue
    echo "--- $log ---"
    tail -80 "$log" 2>/dev/null || true
  done

  echo
  echo "=== qemu wrapper argv ==="
  for log in "$OUT_DIR"/qemu-wrapper-*.argv; do
    [ -e "$log" ] || continue
    echo "--- $log ---"
    cat "$log" 2>/dev/null || true
    echo
  done

  echo
  echo "=== qemu wrapper effective argv ==="
  for log in "$OUT_DIR"/qemu-wrapper-*.effective-argv; do
    [ -e "$log" ] || continue
    echo "--- $log ---"
    cat "$log" 2>/dev/null || true
    echo
  done

  echo
  echo "=== process state ==="
  tail -80 "$PROCESS_LOG" 2>/dev/null || true

  echo
  echo "=== socket state ==="
  tail -120 "$SOCKET_LOG" 2>/dev/null || true

  echo
  echo "=== rootfs/share state ==="
  cat "$OUT_DIR/rootfs-state.log" 2>/dev/null || true

  echo
  echo "=== live rootfs/share state ==="
  tail -260 "$OUT_DIR/rootfs-live-state.log" 2>/dev/null || true

  echo
  echo "=== image state ==="
  cat "$OUT_DIR/image-state.log" 2>/dev/null || true

  echo
  echo "=== live image state ==="
  tail -260 "$OUT_DIR/image-live-state.log" 2>/dev/null || true

  echo
  echo "=== interesting runtime lines ==="
  tail -140 "$INTERESTING_LOG" 2>/dev/null || true
} > "$SUMMARY_LOG"

echo "=== output files ==="
ls -l "$OUT_DIR" /tmp/kata-run.txt 2>/dev/null || true

echo
echo "=== summary ==="
cat "$SUMMARY_LOG" 2>/dev/null || true

echo
echo "=== journal log ==="
echo "$JOURNAL_LOG"
