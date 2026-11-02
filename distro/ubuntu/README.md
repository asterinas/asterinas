<!-- SPDX-License-Identifier: MPL-2.0 -->

# Ubuntu rootfs on Asterinas

This directory builds a bootable Ubuntu 24.04 root filesystem for Asterinas.
The default configuration boots `systemd` to multi-user mode, starts an
autologin root shell on the virtio console, brings up DHCP networking, and
leaves `apt` usable.

The current target is x86_64 with the QEMU device layout used by
`tools/qemu_args.sh`. The design is intentionally user-space only: no kernel
changes are required for the current feature set.

## Why Ubuntu Base and not a cloud image

Asterinas currently mounts the root filesystem through its ext2 driver
(`kernel/core/src/fs/rootfs.rs`). The driver accepts the base ext2 feature set
only:

- incompat: `filetype`
- ro-compat: `sparse_super`, `large_file`, `btree_dir`

Ubuntu cloud images and server ISOs use ext4 with `extents`, `64bit`,
`metadata_csum`, and related features, so they cannot be mounted directly.
An `ubuntu-base` tarball is just a file tree; `build.sh` creates an ext2 image
from it with the features Asterinas supports.

`ubuntu-base` also does not contain `systemd`, `udev`, or `/sbin/init`.
`build.sh --with-systemd` installs them into the staging rootfs with apt.

## Boot flow

```text
Asterinas kernel
  root=/dev/vdd rootfstype=ext2 rw init=/init
        |
        v
  /init (rootfs/overlay/init)
    - mount proc, sysfs, devtmpfs, devpts, /dev/shm, /run, /tmp
    - mount cgroup2 on /sys/fs/cgroup
    - create /run/{dbus,systemd,log,utmp,btmp}
    - create /dev/kmsg if devtmpfs does not provide it
    - exec /sbin/init
        |
        v
  systemd (PID 1)
    - masked/placeholder units for unsupported kernel features
    - systemd-networkd configures eth0 over DHCP
    - asterinas-console.service runs agetty on /dev/hvc0
        |
        v
  root shell on hvc0
```

`/init` plays the same role that the custom NixOS stage-1 init plays for
Asterinas NixOS: it prepares the mounts systemd expects and then hands over.
It is not Ubuntu's initramfs; there are no Linux kernel modules to load.

Two kernel command line switches are available for debugging:

- `aster.nosystemd`: stop after the `/init` mounts and run `/bin/sh`.
- `aster.selftest`: run `/etc/asterinas/selftest.sh` before the shell/systemd.

## Directory layout

| Path | Purpose |
| --- | --- |
| `build.sh` | Extract the tarball, apply the overlay, optionally install systemd, create the ext2 image |
| `overlay/init` | Rootfs init wrapper described above |
| `overlay/etc/systemd/system.conf.d/asterinas.conf` | Disable cgroup accounting and log to the console |
| `overlay/etc/systemd/network/20-asterinas.network` | DHCP configuration for the virtio-net interface |
| `overlay/etc/systemd/system/asterinas-console.service` | Console getty on `/dev/hvc0` |
| `overlay/etc/systemd/system/asterinas-boottest.service` | Optional post-boot test, gated by `aster.boottest` |
| `overlay/etc/pam.d/login` | Minimal PAM stack for the serial console |
| `overlay/etc/apt/apt.conf.d/99asterinas` | IPv4 resolution and apt sandbox workarounds |
| `overlay/etc/asterinas/selftest.sh` | Diagnostics for `aster.selftest` |
| `overlay/etc/asterinas/boottest.sh` | Automated boot/network/apt check |

## Build

The script must run as root because it preserves file ownership while
populating the image. Use the Asterinas development container.

```bash
# Download the input once (the Makefile also prints this command if missing)
wget -O ubuntu-base-24.04.5-base-amd64.tar.gz \
  https://cdimage.ubuntu.com/ubuntu-base/releases/24.04/release/ubuntu-base-24.04.5-base-amd64.tar.gz

# Build the default systemd image
make ubuntu-rootfs
```

The image is written to `test/initramfs/build/ubuntu-rootfs.img`. It is a
4 GiB sparse ext2 image and currently uses about 280 MiB on disk.

Build steps:

1. Extract the tarball with ownership and extended attributes preserved.
2. Copy `overlay/` into the staging rootfs and normalize its ownership.
3. Reset `root` to an empty password and make `pam_loginuid` optional.
4. If requested, mount `/dev`, `/proc`, and `/sys` into the staging rootfs
   and run apt inside a chroot to install:
   `systemd systemd-sysv udev dbus iproute2 ca-certificates`.
5. Mask or replace units that depend on unsupported kernel features.
6. Write `/etc/resolv.conf` and create the ext2 filesystem with
   `mke2fs -t ext2 -F -b 4096 -I 256 -m 0 -d`.
7. Reject the image if an unsupported feature such as `extent`, `64bit`, or
   `metadata_csum` is present.

`--with-systemd` needs network access. Use
`make ubuntu-rootfs UBUNTU_ROOTFS_WITH_SYSTEMD=0` for an offline, shell-only
image.

## Usage

All commands run inside the development container.

```bash
# Build the default systemd image and boot it
make run_ubuntu

# Equivalent explicit form
make UBUNTU_ROOTFS=1 run_kernel

# Automated check; the guest powers off when it finishes
make UBUNTU_ROOTFS=1 CMDLINE="aster.boottest" run_kernel

# Minimal ubuntu-base image without systemd
make ubuntu-rootfs UBUNTU_ROOTFS_WITH_SYSTEMD=0
make UBUNTU_ROOTFS=1 run_kernel

# Debug shell, bypassing systemd
make UBUNTU_ROOTFS=1 CMDLINE="aster.nosystemd" run_kernel
```

The Makefile options are:

| Variable | Default | Meaning |
| --- | --- | --- |
| `UBUNTU_ROOTFS` | `0` | Set to `1` to boot the Ubuntu image instead of the initramfs/Nix rootfs |
| `UBUNTU_ROOTFS_TARBALL` | `$(CURDIR)/ubuntu-base-24.04.5-base-amd64.tar.gz` | Input tarball |
| `UBUNTU_ROOTFS_IMAGE` | `test/initramfs/build/ubuntu-rootfs.img` | Output image |
| `UBUNTU_ROOTFS_SIZE` | `4G` | Sparse image size |
| `UBUNTU_ROOTFS_WITH_SYSTEMD` | `1` | Install systemd with apt |
| `UBUNTU_ROOTFS_DNS` | `10.0.2.3` | Nameserver written to the image |
| `UBUNTU_ROOTFS_INIT` | `/init` | Init path passed to the kernel |

The build options are stored in `ubuntu-rootfs.img.config`; changing an option
rebuilds the image. `tools/qemu_args.sh` attaches the image when `INITRAMFS`
is `off` through the `ROOTFS_IMAGE` environment variable.

## Systemd adaptation details

### Cgroup accounting

Asterinas exposes cgroup2 but writing `memory.max` currently returns `EIO`.
`system.conf.d/asterinas.conf` disables memory, task, IO, CPU, and IP
accounting so unit startup does not fail on those writes. Systemd still logs
`System is tainted: cgroups-missing`, which is expected.

### Masked units

The following units are masked because they need kernel interfaces that are
not available or because static configuration replaces them:

| Unit(s) | Reason |
| --- | --- |
| `systemd-udevd*` | Device monitor/netlink uevent support is incomplete; `/dev` comes from devtmpfs |
| `systemd-journald*` | `/dev/kmsg` is not exposed by devtmpfs; systemd logs to the console instead |
| `dbus.service`, `dbus.socket` | Initially timed out; the runtime directory is still created for future experiments |
| `systemd-resolved`, `systemd-timesyncd` | static `/etc/resolv.conf` is used |
| `systemd-logind`, `systemd-user-sessions` | replaced with oneshot placeholders |
| `user@`, `user-runtime-dir@` | no user session management yet |
| `tmp.mount`, `sys-kernel-config.mount` | mount-state detection through mountinfo is incomplete; `/init` already mounts `/tmp` |
| `getty@tty1`, `serial-getty@hvc0` | replaced by `asterinas-console.service` |
| `systemd-sysusers`, `systemd-initctl` | not needed for the current image |
| `apt-daily*`, `motd-news*`, `e2scrub*`, `fstrim*`, `apparmor` | reduce boot work and failures in a test VM |

### Console login

Stock `serial-getty@.service` has `BindsTo=dev-hvc0.device`. Without udev,
systemd never creates that device unit, so the getty times out. The custom
`asterinas-console.service` sets `TTYPath=/dev/hvc0` directly and starts
`agetty --autologin root`. It also disables tty reset/hangup, which avoid
unsupported ioctls.

### PAM

Ubuntu's login stack marks `pam_loginuid` as `required`, but Asterinas does
not implement `/proc/self/loginuid`. `build.sh` makes the module optional and
the overlay replaces `/etc/pam.d/login` with a minimal permissive stack.
Autologin skips authentication, so this is enough for the console. Password
login and full user sessions are not supported yet.

### Networking and apt

`systemd-networkd` gets an address over DHCP and writes the QEMU nameserver
`10.0.2.3`. `/etc/apt/apt.conf.d/99asterinas` forces IPv4 and relaxes apt
timeouts. It also sets `APT::Sandbox::User "root"` because DNS resolution as
the `_apt` user currently fails on Asterinas. This is a test-VM tradeoff.

## Verification

The automated boottest covers:

- `multi-user.target` and `graphical.target` active
- no failed units
- `asterinas-console.service` active and a root shell on `hvc0`
- `eth0` receives `10.0.2.15/24`
- DNS resolution succeeds
- `apt-get update` populates the package index

`systemctl is-system-running` may print `starting` while the boottest itself
is still running; use `systemctl is-active multi-user.target` for a definitive
check.

## Known limitations

- Only ext2 root filesystems are supported.
- Only the x86_64 QEMU layout is tested; `/dev/vdd` is the rootfs device.
- udev, journald, dbus, logind, and user sessions are disabled.
- `-.mount` is deactivated by systemd because mountinfo events are incomplete.
- `ip route` hangs on a netlink route query; `ip addr`, DHCP, DNS, and apt work.
- Login is root autologin only; there is no password or unprivileged login.
- apt runs as root, as described above.

## Next steps

Useful kernel/user-space follow-ups:

1. Expose `/dev/kmsg` so systemd-journald can be enabled.
2. Implement poll/inotify semantics for `/proc/*/mountinfo` so the standard
   mount units and `-.mount` tracking can be restored.
3. Provide `/proc/self/loginuid` and `/proc/self/sessionid` to allow the stock
   PAM stack and systemd-logind.
4. Complete netlink route queries so tools such as `ip route` work.
5. Investigate the `_apt` DNS failure and remove the apt root sandbox
   workaround.
6. Add ext4 support if Ubuntu images with the default ext4 layout are desired.

## Troubleshooting

- **Emergency mode**: run `systemctl --failed`; a mount unit usually failed.
  Check that `tmp.mount` and `sys-kernel-config.mount` are still masked.
- **Login prints "Permission denied"**: the PAM stack is too strict or
  `/proc/self/loginuid` is being required. Use the provided
  `overlay/etc/pam.d/login`.
- **apt cannot resolve hosts**: check `/etc/resolv.conf`, then
  `getent hosts archive.ubuntu.com`. The image should contain
  `nameserver 10.0.2.3` and the apt IPv4 configuration.
- **`make ubuntu-rootfs` says the tarball is missing**: run the `wget`
  command printed by the Makefile.
- **The image was not rebuilt after changing an option**: remove
  `test/initramfs/build/ubuntu-rootfs.img.config` or the image and rebuild.
