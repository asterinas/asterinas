#!/bin/bash
# SPDX-License-Identifier: MPL-2.0
#
# Build and boot the full Asterinas kernel on AArch64 (QEMU `virt`), running a
# minimal userspace `/init` from an initramfs to demonstrate end-to-end boot,
# ELF loading, and system calls.
#
# Requirements: the pinned Rust nightly with the `aarch64-unknown-none-softfloat`
# target, `cargo-osdk` built with `OSDK_LOCAL_DEV=1`, and `qemu-system-aarch64`.
#
# NOTE: QEMU does not populate the device tree's `linux,initrd-start` for a
# directly booted ELF, so the initramfs is loaded at a fixed address via
# `-device loader` and its location is passed on the kernel command line as
# `initrd=<paddr>,<size>` (parsed by the AArch64 boot code).

set -e

ASTER_DIR="$(cd "$(dirname "$0")/.." && pwd)"
INITRAMFS="${INITRAMFS:-$ASTER_DIR/test/initramfs/build/initramfs.cpio.gz}"
INITRD_ADDR="0x48000000"

echo "==> Building the AArch64 kernel image via cargo-osdk"
( cd "$ASTER_DIR/kernel" && OSDK_TARGET_ARCH=aarch64 \
    cargo osdk build --target-arch aarch64 --scheme aarch64 )

KERNEL="$ASTER_DIR/target/osdk/aster-kernel-osdk-bin.qemu_elf"
INITRD_SIZE="$(stat -c%s "$INITRAMFS")"

echo "==> Booting in QEMU (initramfs @ $INITRD_ADDR, size $INITRD_SIZE)"
exec qemu-system-aarch64 \
    -machine virt,gic-version=2 \
    -cpu cortex-a72 \
    -m 2G \
    -smp 1 \
    -nographic \
    -no-reboot \
    -kernel "$KERNEL" \
    -device "loader,file=$INITRAMFS,addr=$INITRD_ADDR,force-raw=on" \
    -append "console=ttyS0 init=/init initrd=$INITRD_ADDR,$INITRD_SIZE"
