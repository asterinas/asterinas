#!/bin/sh
set -u

OUT_DIR=${KATA_DEBUG_OUT:-/tmp/kata-debug-out}
ADDR=${CONTAINERD_ADDRESS:-/tmp/containerd-debug.sock}
ROOT=${CONTAINERD_ROOT:-/tmp/containerd-root}
STATE=${CONTAINERD_STATE:-/tmp/containerd-state}
CID=${KATA_IMAGE_ID:-kata-image-probe}
IMAGE_TAR=${KATA_IMAGE_TAR:-/etc/kata-debug/busybox.tar}
IMAGE_REF=${KATA_IMAGE_REF:-}
IMAGE_CMD=${KATA_IMAGE_CMD:-/bin/sh}
IMAGE_IMPORT_TIMEOUT=${KATA_IMAGE_IMPORT_TIMEOUT:-120s}
IMAGE_RUN_TIMEOUT=${KATA_IMAGE_RUN_TIMEOUT:-900s}
IMAGE_AUTO_RESOLVE_SH=${KATA_IMAGE_AUTO_RESOLVE_SH:-0}
IMAGE_LIVE_SAMPLES=${KATA_IMAGE_LIVE_SAMPLES:-8}
IMAGE_LIVE_INTERVAL=${KATA_IMAGE_LIVE_INTERVAL:-10}
IMAGE_LIVE_HEAVY=${KATA_IMAGE_LIVE_HEAVY:-0}

mkdir -p "$OUT_DIR"

if [ ! -S "$ADDR" ]; then
  if [ -S /run/containerd/containerd.sock ]; then
    ADDR=/run/containerd/containerd.sock
  else
    echo "no containerd socket found; run kata-debug-start-containerd first" >&2
    exit 1
  fi
fi

log() {
  echo "$*" | tee -a "$OUT_DIR/image-probe.log"
}

run_and_log() {
  label=$1
  shift
  {
    echo
    echo "=== $label ==="
    "$@"
    echo "status:$?"
  } >> "$OUT_DIR/image-probe.log" 2>&1
}

log_path_state() {
  label=$1
  path=$2
  {
    echo
    echo "=== $label ==="
    if [ -e "$path" ] || [ -L "$path" ]; then
      ls -ld "$path"
      if [ -L "$path" ]; then
        readlink "$path"
      fi
    else
      echo "missing: $path"
    fi
  } >> "$OUT_DIR/image-probe.log" 2>&1
}

resolve_image_command() {
  IMAGE_CMD_SOURCE=default

  if [ "$IMAGE_CMD" != "/bin/sh" ]; then
    return
  fi

  sh_path=$(find "$ROOT/io.containerd.snapshotter.v1.overlayfs/snapshots" \
    -maxdepth 8 -path '*/fs/bin/sh' -print 2>/dev/null | head -n1)
  if [ -z "${sh_path:-}" ]; then
    log "image command probe: no snapshot /bin/sh found"
    return
  fi

  sh_target=$(readlink "$sh_path" 2>/dev/null || true)
  log "image command probe: snapshot_sh=$sh_path"
  log "image command probe: snapshot_sh_target=${sh_target:-<not-symlink>}"
  log_path_state "snapshot /bin/sh" "$sh_path"

  case "$sh_target" in
    /*)
      snapshot_root=${sh_path%/bin/sh}
      log_path_state "snapshot absolute sh target" "$snapshot_root$sh_target"
      if [ "$IMAGE_AUTO_RESOLVE_SH" != "1" ]; then
        log "image command probe: leaving /bin/sh unchanged"
        return
      fi
      if [ -e "$snapshot_root$sh_target" ]; then
        IMAGE_CMD=$sh_target
        IMAGE_CMD_SOURCE=resolved-absolute-symlink
        log "image command probe: using rootfs-local target $IMAGE_CMD"
      else
        log "image command probe: target missing under rootfs $snapshot_root$sh_target"
      fi
      ;;
  esac
}

log "containerd address: $ADDR"
log "container id: $CID"
log "image tar: $IMAGE_TAR"
log "image command initial: $IMAGE_CMD"
log "image auto resolve sh: $IMAGE_AUTO_RESOLVE_SH"
log "image import timeout: $IMAGE_IMPORT_TIMEOUT"
log "image run timeout: $IMAGE_RUN_TIMEOUT"
log "image live samples: $IMAGE_LIVE_SAMPLES interval=$IMAGE_LIVE_INTERVAL heavy=$IMAGE_LIVE_HEAVY"

ctr --address "$ADDR" tasks kill "$CID" >/dev/null 2>&1 || true
ctr --address "$ADDR" containers rm "$CID" >/dev/null 2>&1 || true
rm -f /tmp/kata-image-run.txt /tmp/kata-image-import.txt
rm -rf "$STATE/io.containerd.runtime.v2.task/default/$CID" \
  "/run/kata/$CID" \
  "/run/kata-containers/shared/sandboxes/$CID"

if [ ! -r "$IMAGE_TAR" ]; then
  log "missing image tar"
  echo "missing image tar: $IMAGE_TAR" > /tmp/kata-image-run.txt
  echo "exit:125" >> /tmp/kata-image-run.txt
  exit 0
fi

(
  i=0
  while [ "$i" -lt "$IMAGE_LIVE_SAMPLES" ]; do
    {
      echo "=== live $i $(date) ==="
      echo "--- snapshots ---"
      find "$ROOT/io.containerd.snapshotter.v1.overlayfs/snapshots" \
        -maxdepth 3 -ls 2>&1 || true
      echo "--- containerd task state ---"
      find "$STATE/io.containerd.runtime.v2.task/default/$CID" \
        -maxdepth 4 -ls 2>&1 || true
      echo "--- kata state ---"
      find /run/kata /run/kata-containers -maxdepth 5 -ls 2>&1 || true
      if [ "$IMAGE_LIVE_HEAVY" = "1" ]; then
        echo "--- imported images ---"
        ctr --address "$ADDR" images ls 2>&1 || true
        echo "--- snapshot /bin views ---"
        find "$ROOT/io.containerd.snapshotter.v1.overlayfs/snapshots" \
          -path '*/fs/bin' -maxdepth 8 -ls 2>&1 || true
        find "$ROOT/io.containerd.snapshotter.v1.overlayfs/snapshots" \
          -path '*/fs/bin/sh' -maxdepth 8 -ls 2>&1 || true
        echo "--- kata/containerd mounts ---"
        mount 2>&1 | grep -Ei 'kata|containerd|virtio|overlay' || true
        echo "--- mountinfo ---"
        cat /proc/self/mountinfo 2>&1 | grep -Ei 'kata|containerd|virtio|overlay' || true
      fi
    } >> "$OUT_DIR/image-live-state.log" 2>&1
    i=$((i + 1))
    sleep "$IMAGE_LIVE_INTERVAL"
  done
) &
SNAP_PID=$!

timeout "$IMAGE_IMPORT_TIMEOUT" ctr --debug --address "$ADDR" image import "$IMAGE_TAR" \
  > /tmp/kata-image-import.txt 2>&1
IMPORT_STATUS=$?
{
  echo "=== image import output ==="
  cat /tmp/kata-image-import.txt 2>&1 || true
  echo "import_exit:$IMPORT_STATUS"
} >> "$OUT_DIR/image-probe.log"

if [ "$IMPORT_STATUS" -ne 0 ]; then
  kill "$SNAP_PID" >/dev/null 2>&1 || true
  wait "$SNAP_PID" 2>/dev/null || true
  cp /tmp/kata-image-import.txt /tmp/kata-image-run.txt
  echo "exit:$IMPORT_STATUS" >> /tmp/kata-image-run.txt
  exit 0
fi

if [ -z "$IMAGE_REF" ]; then
  IMAGE_REF=$(ctr --address "$ADDR" images ls -q 2>/dev/null \
    | grep -m1 'busybox' || true)
fi
if [ -z "$IMAGE_REF" ]; then
  IMAGE_REF=busybox:latest
fi
log "image ref: $IMAGE_REF"
resolve_image_command
log "image command final: $IMAGE_CMD source=$IMAGE_CMD_SOURCE"
log "image run start: $(date)"

timeout "$IMAGE_RUN_TIMEOUT" ctr --debug \
  --address "$ADDR" \
  run \
  --runtime io.containerd.kata.v2 \
  "$IMAGE_REF" \
  "$CID" \
  "$IMAGE_CMD" -c 'echo HI-IMG; exit 7' \
  > /tmp/kata-image-run.txt 2>&1
STATUS=$?
log "image run exit: $STATUS at $(date)"

kill "$SNAP_PID" >/dev/null 2>&1 || true
wait "$SNAP_PID" 2>/dev/null || true

echo "exit:$STATUS" >> /tmp/kata-image-run.txt
cat /tmp/kata-image-run.txt | tee -a "$OUT_DIR/image-probe.log"

{
  echo "=== image import output ==="
  cat /tmp/kata-image-import.txt 2>&1 || true

  echo
  echo "=== image list ==="
  ctr --address "$ADDR" images ls 2>&1 || true

  echo
  echo "=== snapshot tree ==="
  find "$ROOT/io.containerd.snapshotter.v1.overlayfs/snapshots" \
    -maxdepth 8 -ls 2>&1 || true

  echo
  echo "=== snapshot /bin/sh ==="
  find "$ROOT/io.containerd.snapshotter.v1.overlayfs/snapshots" \
    -path '*/fs/bin/sh' -maxdepth 8 -ls 2>&1 || true
  find "$ROOT/io.containerd.snapshotter.v1.overlayfs/snapshots" \
    -path '*/fs/bin/sh' -maxdepth 8 -exec readlink {} \; 2>&1 || true

  echo
  echo "=== image command resolution ==="
  echo "IMAGE_CMD=$IMAGE_CMD"
  echo "IMAGE_CMD_SOURCE=$IMAGE_CMD_SOURCE"

  echo
  echo "=== containerd task state ==="
  find "$STATE/io.containerd.runtime.v2.task/default/$CID" \
    -maxdepth 8 -ls 2>&1 || true

  echo
  echo "=== oci config ==="
  cat "$STATE/io.containerd.runtime.v2.task/default/$CID/config.json" \
    2>&1 || true

  echo
  echo "=== kata state ==="
  find /run/kata /run/kata-containers -maxdepth 10 -ls 2>&1 || true

  echo
  echo "=== kata shared rootfs direct probes ==="
  for p in \
    "/run/kata-containers/shared/sandboxes/$CID/rw/passthrough/$CID/rootfs" \
    "/run/kata-containers/shared/sandboxes/$CID/ro/passthrough/$CID/rootfs" \
    "/run/kata-containers/shared/containers/passthrough/$CID/rootfs"
  do
    echo "--- $p ---"
    ls -ld "$p" "$p/bin" "$p/bin/sh" "$p/bin/busybox" 2>&1 || true
    readlink "$p/bin/sh" 2>&1 || true
    find "$p" -maxdepth 2 -ls 2>&1 | head -80 || true
  done

  echo
  echo "=== kata/containerd mounts ==="
  mount 2>&1 | grep -Ei 'kata|containerd|virtio|overlay' || true

  echo
  echo "=== mountinfo kata/containerd ==="
  cat /proc/self/mountinfo 2>&1 | grep -Ei 'kata|containerd|virtio|overlay' || true
} > "$OUT_DIR/image-state.log" 2>&1

exit 0
