#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e

SCRIPT_DIR=$(cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd)
ASTERINAS_DIR=$(realpath ${SCRIPT_DIR}/../..)
ASTER_IMAGE_PATH=${ASTERINAS_DIR}/target/nixos/asterinas.img
NIXOS_DISK_SIZE_IN_MB=${NIXOS_DISK_SIZE_IN_MB:-8192}
ISO_IMAGE_PATH=$(realpath ${ASTERINAS_DIR}/target/nixos/iso_image/iso/*.iso)

if [ ! -f "$ISO_IMAGE_PATH" ]; then
    echo "Error: ISO_IMAGE not found!"
    exit 1
fi

rm -f ${ASTER_IMAGE_PATH}
echo "Creating image at ${ASTER_IMAGE_PATH} of size ${NIXOS_DISK_SIZE_IN_MB}MB......"
dd if=/dev/zero of=${ASTER_IMAGE_PATH} bs=1M count=${NIXOS_DISK_SIZE_IN_MB}
echo "Image created successfully!"

QEMU_ARGS="qemu-system-x86_64 \
	-bios /root/ovmf/release/OVMF.fd \
	-cdrom ${ISO_IMAGE_PATH} -boot d \
	-drive if=none,format=raw,id=u0,file=${ASTER_IMAGE_PATH} \
	-device virtio-blk-pci,drive=u0,disable-legacy=on,disable-modern=off \
"

if [ "${ENABLE_KVM}" = "1" ]; then
	QEMU_ARGS="${QEMU_ARGS} -accel kvm"
fi

COMMON_QEMU_ARGS=$(${SCRIPT_DIR}/../qemu_args.sh common 2>/dev/null)
QEMU_ARGS="
	${QEMU_ARGS} \
	${COMMON_QEMU_ARGS} \
"

${QEMU_ARGS}
