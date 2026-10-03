#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

source /etc/profile

# Step 1: run dbus
mkdir -p /var/lib/dbus /usr/share/X11/xorg.conf.d
[ -f /var/lib/dbus/machine-id ] || dbus-uuidgen --ensure=/var/lib/dbus/machine-id

if command -v dbus-launch >/dev/null 2>&1; then
  eval "$(dbus-launch --sh-syntax)"
fi

# On a standard NixOS system, /run/opengl-driver is created and populated
# automatically by systemd-tmpfiles during early boot. The tmpfiles rules
# are responsible for setting up volatile runtime paths under /run and
# linking them to the active OpenGL driver closure (graphics-drivers).
#
# On Asterinas, this path is not available when the desktop startup script
# runs. The graphics-drivers derivation is present in the Nix store, but the
# /run/opengl-driver hierarchy has not been created yet.
#
# To allow Mesa, GBM, and GLX-based applications (e.g. glxinfo, Xorg) to
# locate their runtime driver files, we manually create a symbolic link
# from /run/opengl-driver to the active graphics-drivers store path here.
#
# This is a temporary workaround for debugging and bring-up. Remove it once
# system initialization creates /run/opengl-driver before desktop startup.
DRIVERS_PATH=$(ls -d /nix/store/*-graphics-drivers | head -n1)

if [ -z "$DRIVERS_PATH" ]; then
    echo "graphics-drivers not found"
    exit 1
fi

ln -sf "$DRIVERS_PATH" /run/opengl-driver

mkdir -p /run/user/0
chmod 700 /run/user/0
export XDG_RUNTIME_DIR=/run/user/0

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


# Step 3: run xfce4
export DISPLAY=:0
LOG=/var/log/xfce-session.log
mkdir -p "$(dirname "$LOG")"
: > "$LOG"                 # truncate/create
chmod 600 "$LOG"
nohup xfce4-session >>"$LOG" 2>&1 &
