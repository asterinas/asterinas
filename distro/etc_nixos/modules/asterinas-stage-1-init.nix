# Universal stage-1 init for Asterinas NixOS.
# Shared by both disk and ISO boot paths.
#
# On Asterinas, devtmpfs is not supported, so this script explicitly creates
# /dev device nodes using busybox mknod.  It handles two boot modes:
#   - Disk mode: root= is present → mount root, switch_root
#   - ISO mode:  root= is absent  → exec NixOS stage-2 init directly
{ pkgs }:
pkgs.writeShellScript "stage-1-init" ''
  #!/bin/sh
  # SPDX-License-Identifier: MPL-2.0

  # Create essential /dev nodes.
  # Asterinas does not support devtmpfs, so the NixOS mounts.sh in
  # stage-2 cannot populate /dev via devtmpfs.
  mknod /dev/console c 5 1 2>/dev/null;  chmod 666 /dev/console
  mknod /dev/tty c 5 0 2>/dev/null;     chmod 666 /dev/tty
  mknod /dev/tty0 c 4 0 2>/dev/null;    chmod 622 /dev/tty0
  mknod /dev/ttyS0 c 4 64 2>/dev/null;  chmod 660 /dev/ttyS0
  mknod /dev/hvc0 c 229 0 2>/dev/null;  chmod 666 /dev/hvc0
  mknod /dev/hvc1 c 229 1 2>/dev/null;  chmod 660 /dev/hvc1
  mknod /dev/hvc2 c 229 2 2>/dev/null;  chmod 660 /dev/hvc2
  mknod /dev/hvc3 c 229 3 2>/dev/null;  chmod 660 /dev/hvc3
  mknod /dev/null c 1 3 2>/dev/null;     chmod 666 /dev/null
  mknod /dev/zero c 1 5 2>/dev/null;    chmod 666 /dev/zero
  mknod /dev/full c 1 7 2>/dev/null;    chmod 666 /dev/full
  mknod /dev/random c 1 8 2>/dev/null;  chmod 666 /dev/random
  mknod /dev/urandom c 1 9 2>/dev/null; chmod 666 /dev/urandom
  mknod /dev/ptmx c 5 2 2>/dev/null;    chmod 666 /dev/ptmx
  mknod /dev/tty1 c 4 1 2>/dev/null;    chmod 620 /dev/tty1
  mknod /dev/tty2 c 4 2 2>/dev/null;    chmod 620 /dev/tty2
  mknod /dev/tty3 c 4 3 2>/dev/null;    chmod 620 /dev/tty3
  mknod /dev/tty4 c 4 4 2>/dev/null;    chmod 620 /dev/tty4
  mknod /dev/tty5 c 4 5 2>/dev/null;    chmod 620 /dev/tty5
  mknod /dev/tty6 c 4 6 2>/dev/null;    chmod 620 /dev/tty6
  mkdir -p /dev/pts /dev/shm

  # Mount /proc and /sys (required by NixOS stage-2 and systemd).
  mount -t proc none /proc
  mount -t sysfs none /sys

  # Create symlinks to proc files.
  ln -sfn /proc/self/fd /dev/fd
  ln -sfn /proc/self/fd/0 /dev/stdin
  ln -sfn /proc/self/fd/1 /dev/stdout
  ln -sfn /proc/self/fd/2 /dev/stderr

  NEW_ROOT=""
  NEW_INIT=""
  BREAK=""
  ARGS=""

  for arg in "$@"; do
    case "$arg" in
      root=*)
        NEW_ROOT=''${arg#root=}
        ;;
      init=*)
        NEW_INIT=''${arg#init=}
        ;;
      rd.break=*)
        BREAK=''${arg#rd.break=}
        ;;
      *)
        ARGS="$ARGS $arg"
        ;;
    esac
  done

  if [ "$BREAK" = "1" ]; then
    echo "Breaking into initramfs shell..."
    exec /bin/sh
  fi

  if [ -n "$NEW_ROOT" ]; then
    # Disk mode: mount root filesystem and switch_root.
    mkdir /sysroot
    mount -t ext2 "$NEW_ROOT" /sysroot
    mount -t proc none /sysroot/proc
    mkdir -p /sysroot/run/initramfs/dev
    mount -o bind /dev /sysroot/run/initramfs/dev
    mount --move /dev /sysroot/dev
    exec switch_root /sysroot "$NEW_INIT" $ARGS
  else
    # ISO mode or Fallback: the root is a new ramfs to allow pivot_root.
    TARGET_INIT=""
    if [ -n "$NEW_INIT" ]; then
      TARGET_INIT="$NEW_INIT"
    else
      for candidate in /nix/store/*/stage-2-init; do
        if [ -f "$candidate" ] && [ -x "$candidate" ]; then
          TARGET_INIT="$candidate"
          break
        fi
      done
      if [ -z "$TARGET_INIT" ]; then
        for candidate in /nix/store/*/init; do
          if [ -f "$candidate" ] && [ -x "$candidate" ]; then
            if [ "$(readlink -f "$candidate" 2>/dev/null)" != "/bin/busybox" ]; then
              TARGET_INIT="$candidate"
              break
            fi
          fi
        done
      fi
    fi

    if [ -z "$TARGET_INIT" ]; then
      echo "Error: no suitable init found."
      exit 1
    fi

    echo "ISO Mode: preparing ramfs root at /sysroot..."
    mkdir -p /sysroot
    mount -t ramfs none /sysroot

    echo "Copying files to ramfs root..."
    mkdir -p /sysroot/proc /sysroot/sys /sysroot/dev /sysroot/run /sysroot/var /sysroot/tmp
    # Create directories and files that PAM (login) expects on a real
    # filesystem.  Without these, pam_unix.so and pam_lastlog.so fail
    # and the login binary exits with "Error in service module".
    mkdir -p /sysroot/var/log /sysroot/var/run
    touch /sysroot/var/log/lastlog /sysroot/var/run/utmp /sysroot/var/log/wtmp
    cp -a /bin /etc /lib /lib64 /nix /usr /sysroot/

    echo "Moving /dev and mounting /proc and /sys..."
    mkdir -p /sysroot/run/initramfs/dev
    mount -o bind /dev /sysroot/run/initramfs/dev
    mount --move /dev /sysroot/dev
    mount -t proc none /sysroot/proc
    mount -t sysfs none /sysroot/sys

    echo "Switching root to ramfs and executing $TARGET_INIT..."
    exec switch_root /sysroot "$TARGET_INIT" $ARGS
  fi
''
