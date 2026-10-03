#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0

set -euo pipefail

# Wait for Xorg and its GLX server to become available. Keep the final output
# so that later checks can reuse it and failures retain useful diagnostics.
glxinfo_ready=false
for attempt in $(seq 1 30); do
    if timeout 2s env DISPLAY=:0 glxinfo -B >/tmp/drm-glxinfo.txt 2>&1; then
        glxinfo_ready=true
        break
    fi
    sleep 1
done
if [ "$glxinfo_ready" != true ]; then
    cat /tmp/drm-glxinfo.txt
    exit 1
fi

# Verify that DRM exposed its primary device node and Mesa created a direct
# rendering context through the X server.
test -c /dev/dri/card0
grep -Fqx 'direct rendering: Yes' /tmp/drm-glxinfo.txt
grep -F 'OpenGL vendor string:' /tmp/drm-glxinfo.txt | grep -Fq Mesa

# Verify that Xorg detected a connected output and configured a display mode.
DISPLAY=:0 xrandr --query \
    | grep -Eq '^\S+ connected(?: primary)? [0-9]+x[0-9]+\+[0-9]+\+[0-9]+'

# Verify that Xorg has the primary DRM device open, rather than only serving a
# software-only display without accessing the DRM device.
xorg_pid=$(pgrep -o -x Xorg || true)
test -n "$xorg_pid"
for fd in /proc/"$xorg_pid"/fd/*; do
    if [ "$(readlink "$fd" 2>/dev/null || true)" = /dev/dri/card0 ]; then
        echo DRM_XORG_MESA_SMOKE_OK
        exit 0
    fi
done

exit 1
