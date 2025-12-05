#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

NIXOS_DIR=$(realpath $1)
ASTERINAS_DISK=${ASTERINAS_DISK:-"${NIXOS_DIR}/asterinas.img"}
QEMU_ARGS="qemu-system-x86_64 \
	-bios /root/ovmf/release/OVMF.fd \
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
