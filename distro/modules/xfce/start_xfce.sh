#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

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
