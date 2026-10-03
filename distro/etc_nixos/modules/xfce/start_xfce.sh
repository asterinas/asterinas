#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

source /etc/profile

# This root desktop starts without a login session to provision XDG_RUNTIME_DIR.
# Provide its runtime directory before starting D-Bus so activated services inherit it.
# Remove this workaround once login sessions manage it.
mkdir -p /run/user/0
chmod 700 /run/user/0
export XDG_RUNTIME_DIR=/run/user/0
export DISPLAY=:0

# Step 1: run dbus
mkdir -p /var/lib/dbus /usr/share/X11/xorg.conf.d
[ -f /var/lib/dbus/machine-id ] || dbus-uuidgen --ensure=/var/lib/dbus/machine-id

if command -v dbus-launch >/dev/null 2>&1; then
  eval "$(dbus-launch --sh-syntax)"
fi

# Step 2: run Xorg
XKB_DATA="/run/current-system/sw/share/X11/xkb"
MODULE_PATH="/run/current-system/sw/lib/xorg/modules"

nohup Xorg :0 vt1 \
  -modulepath "$MODULE_PATH" \
  -xkbdir "$XKB_DATA" \
  -logverbose 0 \
  -logfile /var/log/xorg_debug.log \
  -novtswitch \
  -keeptty \
  -keyboard keyboard \
  -pointer mouse0 \
  > /var/log/xorg.log 2>&1 &


# Step 3: initialize Xfce's desktop and D-Bus activation environment.
LOG=/var/log/xfce-session.log
mkdir -p "$(dirname "$LOG")"
: > "$LOG"                 # truncate/create
chmod 600 "$LOG"
# The packaged xinitrc is not executable, so run it through the shell.
nohup "@runtime_shell@" "@xfce_xinitrc@" >>"$LOG" 2>&1 &
