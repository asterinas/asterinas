#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

# Step 1: run dbus
mkdir -p /var/lib/dbus /usr/share/X11/xorg.conf.d
[ -f /var/lib/dbus/machine-id ] || dbus-uuidgen --ensure=/var/lib/dbus/machine-id

if command -v dbus-launch >/dev/null 2>&1; then
  eval "$(dbus-launch --sh-syntax)"
fi

# Step 2: run Xorg
XKB_DATA="/run/current-system/sw/share/X11/xkb"
MODULE_PATH="/run/current-system/sw/lib/xorg/modules"

nohup Xorg :0 \
  -modulepath "$MODULE_PATH" \
  -xkbdir "$XKB_DATA" \
  -logverbose 0 \
  -logfile /var/log/xorg_debug.log \
  -novtswitch \
  -keeptty \
  -keyboard keyboard \
  -pointer mouse0 \
  > /var/log/xorg.log 2>&1 &

# Step 3: run xfce4
export DISPLAY=:0
LOG=/var/log/xfce-session.log
mkdir -p "$(dirname "$LOG")"
: > "$LOG"                 # truncate/create
chmod 600 "$LOG"
nohup xfce4-session >>"$LOG" 2>&1 &

# Step 4: test Xorg and xfce4 are running
timeout=60
echo "Waiting for Xorg and xfce4-session to start (timeout: ${timeout}s)..."

is_x_running() {
  [ -S /tmp/.X11-unix/X0 ]
}

is_xfce_running() {
  # Check that the core XFCE components are running.
  pgrep -f "[x]fce4-session" >/dev/null 2>&1 \
  && pgrep -f "[x]fwm4" >/dev/null 2>&1 \
  && pgrep -f "[x]fce4-panel" >/dev/null 2>&1 \
  && pgrep -f "[x]fdesktop" >/dev/null 2>&1
}

while [ "$timeout" -gt 0 ]; do
    if is_x_running && is_xfce_running; then
        echo "Success: Both X and key XFCE components are running."
        break
    fi

    sleep 2
    timeout=$((timeout - 2))
done

if [ "$timeout" -le 0 ]; then
    echo "Error: Timed out waiting for XFCE session to start." >&2
    echo "--- DUMPING DIAGNOSTIC INFO ---" >&2

    echo "[ps aux]"
    ps aux || true

    echo "[Xorg log]"
    tail -n 50 /var/log/xorg.log || true

    echo "[XFCE session log]"
    tail -n 50 /var/log/xfce-session.log || true

    exit 1
fi

echo "Test xfce succeeds"
