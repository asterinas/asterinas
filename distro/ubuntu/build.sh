#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# Build an ext2 rootfs image from an Ubuntu Base tarball.
#
# This script is meant to run as root inside the Asterinas development
# container, because it must preserve file ownership while populating the
# ext2 image. Use `--with-systemd` to install systemd and udev through apt;
# that mode additionally needs a privileged container so it can bind-mount
# /dev, /proc, and /sys into the staging rootfs for the package scripts.
#
# Examples:
#   bash distro/ubuntu/build.sh \
#       --tarball ubuntu-base-24.04.5-base-amd64.tar.gz \
#       --output test/initramfs/build/ubuntu-rootfs.img
#
#   bash distro/ubuntu/build.sh \
#       --tarball ubuntu-base-24.04.5-base-amd64.tar.gz \
#       --output test/initramfs/build/ubuntu-rootfs.img \
#       --with-systemd

set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
DEFAULT_OVERLAY="$SCRIPT_DIR/overlay"

TARBALL=""
OUTPUT=""
SIZE="4G"
OVERLAY="$DEFAULT_OVERLAY"
STAGE=""
WITH_SYSTEMD=0
DNS_SERVER="${DNS_SERVER:-10.0.2.3}"
KEEP_STAGE=0

usage() {
    cat <<'EOF'
Usage: build.sh --tarball <ubuntu-base.tar.gz> --output <rootfs.img> [options]

Options:
  --size <size>       Image size passed to truncate (default: 4G)
  --overlay <dir>     Overlay directory copied on top of the rootfs
                      (default: distro/ubuntu/overlay)
  --stage <dir>       Use a specific staging directory instead of a temporary one
  --with-systemd      Install systemd, systemd-sysv, udev, and dbus with apt
  --dns <addr>        Nameserver written to /etc/resolv.conf (default: 10.0.2.3)
  --keep-stage        Keep the temporary staging directory after the build
  -h, --help          Show this help

The script must run as root. `--with-systemd` requires a privileged container
so the package maintainer scripts can see /dev, /proc, and /sys.
EOF
}

die() {
    echo "error: $*" >&2
    exit 1
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --tarball)
            TARBALL="$2"
            shift 2
            ;;
        --output)
            OUTPUT="$2"
            shift 2
            ;;
        --size)
            SIZE="$2"
            shift 2
            ;;
        --overlay)
            OVERLAY="$2"
            shift 2
            ;;
        --stage)
            STAGE="$2"
            shift 2
            ;;
        --with-systemd)
            WITH_SYSTEMD=1
            shift
            ;;
        --dns)
            DNS_SERVER="$2"
            shift 2
            ;;
        --keep-stage)
            KEEP_STAGE=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            die "unknown argument: $1"
            ;;
    esac
done

[ -n "$TARBALL" ] || die "--tarball is required"
[ -n "$OUTPUT" ] || die "--output is required"
[ -f "$TARBALL" ] || die "tarball not found: $TARBALL"
[ -d "$OVERLAY" ] || die "overlay directory not found: $OVERLAY"

if [ "$(id -u)" -ne 0 ]; then
    die "this script must run as root; use the Asterinas development container"
fi

TARBALL=$(realpath "$TARBALL")
OUTPUT=$(realpath -m "$OUTPUT")
OVERLAY=$(realpath "$OVERLAY")

if [ -n "$STAGE" ]; then
    STAGE=$(realpath -m "$STAGE")
    if [ -e "$STAGE" ] && [ -n "$(ls -A "$STAGE" 2>/dev/null || true)" ]; then
        die "staging directory is not empty: $STAGE"
    fi
    mkdir -p "$STAGE"
    CLEAN_STAGE=0
else
    STAGE=$(mktemp -d "${TMPDIR:-/tmp}/asterinas-ubuntu-rootfs.XXXXXX")
    CLEAN_STAGE=1
fi

cleanup() {
    if [ "$CLEAN_STAGE" = 1 ] && [ "$KEEP_STAGE" = 0 ]; then
        rm -rf "$STAGE"
    fi
}
trap cleanup EXIT

echo "[ubuntu-rootfs] staging rootfs at $STAGE"
tar --xattrs --xattrs-include='*' --numeric-owner -xpf "$TARBALL" -C "$STAGE"

[ -x "$STAGE/usr/bin/bash" ] || die "the tarball does not look like an Ubuntu rootfs"
[ -d "$STAGE/etc" ] || die "the tarball does not contain /etc"

echo "[ubuntu-rootfs] applying overlay from $OVERLAY"
cp -a "$OVERLAY"/. "$STAGE"/

# The overlay lives in the source tree and is owned by the developer, while
# the rootfs must be owned by root. Normalize only the overlay entries; the
# base tarball may contain non-root owners such as the apt sandbox user.
while IFS= read -r entry; do
    chown -R 0:0 "$STAGE/$entry"
done < <(cd "$OVERLAY" && find . -mindepth 1 -maxdepth 1 -printf '%P\n')

chmod 755 "$STAGE/init"
for script in \
    "$STAGE/etc/asterinas/selftest.sh" \
    "$STAGE/etc/asterinas/boottest.sh"; do
    [ -f "$script" ] && chmod 755 "$script"
done

mkdir -p \
    "$STAGE/proc" \
    "$STAGE/sys" \
    "$STAGE/run" \
    "$STAGE/tmp" \
    "$STAGE/dev/pts" \
    "$STAGE/dev/shm" \
    "$STAGE/sys/fs/cgroup" \
    "$STAGE/root" \
    "$STAGE/var/log"

chmod 1777 "$STAGE/tmp" "$STAGE/dev/shm"
printf 'asterinas-ubuntu\n' > "$STAGE/etc/hostname"
cat > "$STAGE/etc/hosts" <<'EOF'
127.0.0.1 localhost
127.0.1.1 asterinas-ubuntu
EOF

# The first userspace process is root. An empty password lets agetty's
# --autologin hand the console to a shell without an external password.
if [ -f "$STAGE/etc/shadow" ]; then
    sed -i 's/^root:[^:]*:/root::/' "$STAGE/etc/shadow"
fi

# Ubuntu's login PAM stack makes pam_loginuid required. It needs
# /proc/self/loginuid, which Asterinas does not implement yet, so login aborts
# with "Permission denied" after autologin. Make the module optional.
if [ -f "$STAGE/etc/pam.d/login" ]; then
    sed -i -E \
        's/^([[:space:]]*session[[:space:]]+)required([[:space:]]+pam_loginuid\.so)/\1optional\2/' \
        "$STAGE/etc/pam.d/login"
fi

mask_unit() {
    local unit=$1
    mkdir -p "$(dirname "$STAGE/etc/systemd/system/$unit")"
    ln -sfn /dev/null "$STAGE/etc/systemd/system/$unit"
}

# Keep the first Ubuntu systemd boot small and predictable. These units cover
# the components that either need kernel features Asterinas does not provide
# yet or that we replace with static configuration (udev -> devtmpfs,
# resolved/timesyncd -> /etc/resolv.conf, logind -> no user sessions).
MASKED_UNITS="
systemd-udevd.service
systemd-udevd-control.socket
systemd-udevd-kernel.socket
systemd-udev-trigger.service
systemd-udev-settle.service
systemd-resolved.service
systemd-timesyncd.service
systemd-random-seed.service
user@.service
user-runtime-dir@.service
getty@tty1.service
serial-getty@hvc0.service
dbus.service
dbus.socket
systemd-journald.service
systemd-journald.socket
systemd-journald-dev-log.socket
systemd-journald-audit.socket
systemd-journald-varlink@.socket
systemd-journald@.service
systemd-journald@.socket
systemd-journal-flush.service
systemd-journal-catalog-update.service
systemd-sysusers.service
systemd-initctl.service
systemd-initctl.socket
apparmor.service
apt-daily.service
apt-daily-upgrade.service
apt-daily.timer
apt-daily-upgrade.timer
motd-news.service
motd-news.timer
e2scrub_all.timer
e2scrub_reap.service
fstrim.timer
unattended-upgrades.service
"
for unit in $MASKED_UNITS; do
    mask_unit "$unit"
done

# /init already mounts these, and systemd's mount-state detection relies on
# libmount events that Asterinas' procfs does not provide yet. Masking the
# redundant mount units keeps local-fs.target from failing.
for unit in tmp.mount sys-kernel-config.mount; do
    mask_unit "$unit"
done

# systemd-logind and systemd-user-sessions are not needed for a single root
# console. NixOS uses placeholders instead of masking them, so do the same:
# other units order themselves after these services.
mkdir -p \
    "$STAGE/etc/systemd/system/getty.target.wants" \
    "$STAGE/etc/systemd/system.conf.d"

for unit in systemd-logind.service systemd-user-sessions.service; do
    cat > "$STAGE/etc/systemd/system/$unit" <<'EOF'
[Unit]
Description=Asterinas placeholder

[Service]
Type=oneshot
ExecStartPre=-/bin/rm -f /run/nologin
ExecStart=/bin/true
RemainAfterExit=yes
EOF
done

# Make the virtio console the primary login. Ubuntu's getty generator does
# not reliably instantiate hvc0 when udev is unavailable, and the stock
# serial-getty unit binds to a dev-hvc0.device unit that systemd cannot create
# without udev. Use our own unit with an explicit TTYPath instead.
mkdir -p "$STAGE/etc/systemd/system/multi-user.target.wants"
ln -sfn /etc/systemd/system/asterinas-console.service \
    "$STAGE/etc/systemd/system/multi-user.target.wants/asterinas-console.service"
ln -sfn /etc/systemd/system/asterinas-boottest.service \
    "$STAGE/etc/systemd/system/multi-user.target.wants/asterinas-boottest.service"

install_systemd() {
    local stage=$1

    echo "[ubuntu-rootfs] installing systemd with apt"

    # apt inside the chroot needs a working resolver. The final image gets the
    # QEMU user-net nameserver again after the install.
    cp -L /etc/resolv.conf "$stage/etc/resolv.conf"

    mount --rbind /dev "$stage/dev"
    mount -t proc proc "$stage/proc"
    mount -t sysfs sysfs "$stage/sys"
    cleanup_mounts() {
        umount -R "$stage/dev" 2>/dev/null || true
        umount "$stage/proc" 2>/dev/null || true
        umount "$stage/sys" 2>/dev/null || true
    }
    trap cleanup_mounts RETURN

    cat > "$stage/usr/sbin/policy-rc.d" <<'EOF'
#!/bin/sh
exit 101
EOF
    chmod +x "$stage/usr/sbin/policy-rc.d"

    chroot "$stage" /usr/bin/env \
        DEBIAN_FRONTEND=noninteractive \
        SYSTEMD_OFFLINE=1 \
        apt-get update
    chroot "$stage" /usr/bin/env \
        DEBIAN_FRONTEND=noninteractive \
        SYSTEMD_OFFLINE=1 \
        apt-get install -y --no-install-recommends \
            systemd systemd-sysv udev dbus iproute2 ca-certificates

    rm -f "$stage/usr/sbin/policy-rc.d"
    rm -rf "$stage/var/lib/apt/lists"/*
    rm -f "$stage"/var/cache/apt/archives/*.deb

    [ -x "$stage/usr/lib/systemd/systemd" ] \
        || die "systemd was not installed into the staging rootfs"

    # systemd-sysv provides this link, but make it deterministic for images
    # built with a different package layout.
    ln -sfn /lib/systemd/systemd "$stage/sbin/init"

    # The package enables getty@tty1, which cannot run on Asterinas yet.
    # Keep the serial console as the primary login instead.
    rm -f "$stage/etc/systemd/system/getty.target.wants/getty@tty1.service"
}

if [ "$WITH_SYSTEMD" = 1 ]; then
    install_systemd "$STAGE"
fi

printf 'nameserver %s\n' "$DNS_SERVER" > "$STAGE/etc/resolv.conf"

echo "[ubuntu-rootfs] building $OUTPUT"
mkdir -p "$(dirname "$OUTPUT")"
rm -f "$OUTPUT"
truncate -s "$SIZE" "$OUTPUT"
mke2fs \
    -t ext2 \
    -F \
    -b 4096 \
    -I 256 \
    -m 0 \
    -L ubuntu-rootfs \
    -d "$STAGE" \
    "$OUTPUT"

FEATURES=$(dumpe2fs -h "$OUTPUT" 2>/dev/null \
    | awk -F: '/Filesystem features/ {gsub(/^[ \t]+/, "", $2); print $2}')
echo "[ubuntu-rootfs] ext2 features: $FEATURES"

for feature in extent 64bit metadata_csum flex_bg dir_nlink extra_isize journal; do
    if echo " $FEATURES " | grep -q " $feature "; then
        die "unsupported ext2 feature '$feature' is enabled in $OUTPUT"
    fi
done

echo "[ubuntu-rootfs] done: $OUTPUT"
