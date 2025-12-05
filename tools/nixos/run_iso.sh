#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

NIXOS_DIR=$(realpath $1)
ISO_IMAGE=$(realpath ${NIXOS_DIR}/iso_image/iso/*.iso)
ASTERINAS_DISK=${ASTERINAS_DISK:-"${NIXOS_DIR}/asterinas.img"}
NIXOS_DISK_SIZE_IN_MB=${NIXOS_DISK_SIZE_IN_MB:-"8196"}

if [ ! -f "$ISO_IMAGE" ]; then
    echo "Error: ISO not found in ${NIXOS_DIR}/iso_image/iso"
    exit 1
fi

rm -f $ASTERINAS_DISK
dd if=/dev/zero of=$ASTERINAS_DISK bs=1M count=$NIXOS_DISK_SIZE_IN_MB
sync

QEMU_ARGS="qemu-system-x86_64 \
	-bios /root/ovmf/release/OVMF.fd \
	-cdrom ${ISO_IMAGE} -boot d \
	-drive if=none,format=raw,id=u0,file=${ASTERINAS_DISK} \
	-device virtio-blk-pci,drive=u0,disable-legacy=on,disable-modern=off \
"

if [ "${ENABLE_KVM}" = "1" ]; then
	QEMU_ARGS="${QEMU_ARGS} -accel kvm"
fi

COMMON_QEMU_ARGS=$(${NIXOS_DIR}/../../tools/qemu_args.sh common 2>/dev/null)
QEMU_ARGS="
	${QEMU_ARGS} \
	${COMMON_QEMU_ARGS} \
"

${QEMU_ARGS}
