#!/bin/sh
# SPDX-License-Identifier: MPL-2.0
#
# Small smoke test executed by /init when the kernel command line contains
# aster.selftest. It intentionally uses only utilities from ubuntu-base.

echo "[ubuntu-selftest] os-release:"
cat /etc/os-release 2>/dev/null || true
echo "[ubuntu-selftest] uname:"
uname -a 2>/dev/null || true
echo "[ubuntu-selftest] mounts:"
cat /proc/mounts 2>/dev/null || true
echo "[ubuntu-selftest] /dev:"
ls -l /dev 2>/dev/null | head -n 20 || true
echo "[ubuntu-selftest] init:"
ls -l /sbin/init /usr/lib/systemd/systemd 2>/dev/null || true
if command -v systemctl >/dev/null 2>&1; then
    systemctl is-system-running 2>/dev/null || true
fi
echo "[ubuntu-selftest] cgroup controllers:"
cat /sys/fs/cgroup/cgroup.controllers 2>/dev/null || true
cat /sys/fs/cgroup/cgroup.subtree_control 2>/dev/null || true
echo "[ubuntu-selftest] /dev/kmsg:"
ls -l /dev/kmsg 2>/dev/null || true
echo "[ubuntu-selftest] loginuid/nologin:"
ls -l /proc/self/loginuid /proc/self/sessionid /run/nologin /run/utmp 2>&1 || true
cat /proc/self/loginuid 2>&1 || true
echo "[ubuntu-selftest] bash:"
/bin/bash -c 'echo bash-ok; id; echo PATH=$PATH' 2>&1 || true
echo "[ubuntu-selftest] login -f root:"
timeout 5 /bin/login -f root </dev/null 2>&1 || true
echo "[ubuntu-selftest] su:"
timeout 5 /bin/su - root -c id 2>&1 || true
echo "[ubuntu-selftest] journald:"
timeout 3 env SYSTEMD_LOG_TARGET=console SYSTEMD_LOG_LEVEL=debug \
    /usr/lib/systemd/systemd-journald 2>&1 | head -n 40 || true
echo "[ubuntu-selftest] dbus:"
timeout 3 /usr/bin/dbus-daemon --system --nofork --nopidfile 2>&1 | head -n 20 || true
echo "[ubuntu-selftest] sysusers:"
/usr/lib/systemd/systemd-sysusers 2>&1 | head -n 20 || true
echo "[ubuntu-selftest] done"
