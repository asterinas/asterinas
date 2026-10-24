# SPDX-License-Identifier: MPL-2.0
# The closure is unpacked once by the kernel and retained as the overlay's lower layer.
{
  pkgs,
  system,
  busybox,
}:
pkgs.writeScript "asterinas-iso-init" ''
  #!${busybox}/bin/sh
  set -eu
  export PATH=${busybox}/bin

  mkdir -p /rw-root /sysroot
  mount -t tmpfs -o mode=0755 tmpfs /rw-root
  mkdir -p /rw-root/upper /rw-root/work
  mount -t overlay overlay -o lowerdir=/iso-root,upperdir=/rw-root/upper,workdir=/rw-root/work /sysroot

  # Stage 2 reads /proc/cmdline and uses /dev/fd before activation.
  mount -t proc proc /sysroot/proc
  mount -t sysfs sysfs /sysroot/sys
  mount -t tmpfs -o mode=0755 tmpfs /sysroot/run
  mount -t devtmpfs devtmpfs /sysroot/dev
  mkdir -p /sysroot/dev/pts
  mount -t devpts devpts /sysroot/dev/pts
  chmod 0666 /sysroot/dev/pts/ptmx
  ln -sfn pts/ptmx /sysroot/dev/ptmx
  ln -sfn /proc/self/fd /sysroot/dev/fd
  ln -sfn /proc/self/fd/0 /sysroot/dev/stdin
  ln -sfn /proc/self/fd/1 /sysroot/dev/stdout
  ln -sfn /proc/self/fd/2 /sysroot/dev/stderr

  # switch_root would delete the initramfs files backing the lower layer.
  # chroot retains them and keeps PID 1 for the NixOS stage-2 init.
  exec chroot /sysroot ${system}/init "$@"
''
