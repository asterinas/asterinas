#!/bin/sh
# SPDX-License-Identifier: MPL-2.0
#
# Automated post-boot check. Enabled only when the kernel command line
# contains aster.boottest.

exec >/dev/hvc0 2>&1

echo "[boottest] start"
cat /etc/os-release 2>/dev/null | head -n 2 || true
uname -a 2>/dev/null || true
id 2>/dev/null || true
echo "[boottest] systemctl:"
systemctl is-active multi-user.target 2>/dev/null || true
systemctl is-active graphical.target 2>/dev/null || true
echo "[boottest] failed units:"
systemctl list-units --failed --no-legend 2>/dev/null || true
echo "[boottest] getty:"
systemctl is-active asterinas-console.service 2>/dev/null || true
echo "[boottest] network:"
ip addr 2>/dev/null || true
echo "[boottest] routes:"
timeout 5 ip route 2>/dev/null || echo "route-query-timeout"
echo "[boottest] resolv.conf:"
cat /etc/resolv.conf 2>/dev/null || true
echo "[boottest] dns:"
if timeout 10 getent hosts archive.ubuntu.com >/dev/null 2>&1; then
    echo "dns-ok"
else
    echo "dns-failed"
fi
echo "[boottest] apt:"
if timeout 60 apt-get update -qq; then
    if timeout 10 apt-cache show bash >/dev/null 2>&1; then
        echo "apt-index-ok"
    else
        echo "apt-index-missing"
    fi
else
    echo "apt-update-failed"
fi
echo "[boottest] done"
poweroff -f
