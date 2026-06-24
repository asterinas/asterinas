#!/bin/sh
set -u

OUT_DIR=${KATA_DEBUG_OUT:-/tmp/kata-debug-out}
WAIT_SECONDS=${KATA_SERIAL_WAIT_SECONDS:-180}
READ_SECONDS=${KATA_SERIAL_READ_SECONDS:-120}

mkdir -p "$OUT_DIR"
LOG="$OUT_DIR/qemu-serial.log"
rm -f "$LOG"

echo "watching for Kata nested QEMU console.sock" | tee -a "$LOG"
echo "wait=${WAIT_SECONDS}s read=${READ_SECONDS}s" | tee -a "$LOG"

end=$(( $(date +%s) + WAIT_SECONDS ))
last_sock=""

while [ "$(date +%s)" -lt "$end" ]; do
  SOCK=$(find /run /tmp -name console.sock -type s -printf '%T@ %p\n' 2>/dev/null \
    | sort -nr \
    | awk 'NR==1 {print $2}')

  if [ -n "${SOCK:-}" ]; then
    if [ "$SOCK" != "$last_sock" ]; then
      echo "console.sock=$SOCK" | tee -a "$LOG"
      last_sock="$SOCK"
    fi

    if command -v socat >/dev/null 2>&1; then
      timeout "$READ_SECONDS"s socat - UNIX-CONNECT:"$SOCK" >> "$LOG" 2>&1
      rc=$?
    elif command -v nc >/dev/null 2>&1; then
      timeout "$READ_SECONDS"s nc -U "$SOCK" >> "$LOG" 2>&1
      rc=$?
    else
      echo "no socat/nc in guest" | tee -a "$LOG"
      exit 0
    fi

    echo "serial-connect-exit:$rc" | tee -a "$LOG"
    if [ "$rc" = "0" ] || [ "$rc" = "124" ]; then
      exit 0
    fi
  fi

  sleep 0.2
done

echo "console.sock not found or not readable before timeout" | tee -a "$LOG"
exit 0
