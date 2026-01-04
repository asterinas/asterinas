#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

# Run a NixOS installation or NixOS ISO installer image built by the root Makefile inside a VM
#
# Usage: ./run.sh [nixos | iso]

set -e

usage() {
    echo "Usage: $0 [nixos | iso]"
    exit 1
}

if [ "$#" -ne 1 ]; then
    usage
fi

MODE=$1
SCRIPT_DIR=$(dirname "$0")
ASTERINAS_DIR=$(realpath "${SCRIPT_DIR}/../..")

# Base QEMU arguments
BASE_QEMU_ARGS="qemu-system-x86_64 \
    -bios /root/ovmf/release/OVMF.fd \
"

# Mode-specific QEMU arguments
case "$MODE" in
    nixos)
        NIXOS_DIR="${ASTERINAS_DIR}/target/nixos"
        QEMU_ARGS="${BASE_QEMU_ARGS} \
            -drive if=none,format=raw,id=u0,file=${NIXOS_DIR}/asterinas.img \
            -device virtio-blk-pci,drive=u0,disable-legacy=on,disable-modern=off \
        "
        ;;
    iso)
        ASTER_IMAGE_PATH=${ASTERINAS_DIR}/target/nixos/asterinas.img
        NIXOS_DISK_SIZE_IN_MB=${NIXOS_DISK_SIZE_IN_MB:-8192}
        ISO_IMAGE_PATH=$(find "${ASTERINAS_DIR}/target/nixos/iso_image/iso" -name "*.iso" | head -n 1)

        if [ ! -f "$ISO_IMAGE_PATH" ]; then
            echo "Error: ISO_IMAGE not found!"
            exit 1
        fi

        rm -f "${ASTER_IMAGE_PATH}"
        echo "Creating image at ${ASTER_IMAGE_PATH} of size ${NIXOS_DISK_SIZE_IN_MB}MB......"
        dd if=/dev/zero of="${ASTER_IMAGE_PATH}" bs=1M count=${NIXOS_DISK_SIZE_IN_MB} status=none
        echo "Image created successfully!"

        QEMU_ARGS="${BASE_QEMU_ARGS} \
            -cdrom ${ISO_IMAGE_PATH} -boot d \
            -drive if=none,format=raw,id=u0,file=${ASTER_IMAGE_PATH} \
            -device virtio-blk-pci,drive=u0,disable-legacy=on,disable-modern=off \
        "
        ;;
    *)
        usage
        ;;
esac

if [ "${ENABLE_KVM}" = "1" ]; then
    QEMU_ARGS="${QEMU_ARGS} -accel kvm"
fi

COMMON_QEMU_ARGS=$(${ASTERINAS_DIR}/tools/qemu_args.sh common 2>/dev/null)
QEMU_ARGS="
    ${QEMU_ARGS} \
    ${COMMON_QEMU_ARGS} \
"

# The kernel uses a specific value to signal a successful shutdown via the
# isa-debug-exit device.
KERNEL_SUCCESS_EXIT_CODE=16 # 0x10 in hexadecimal
# QEMU translates the value written to the isa-debug-exit device into a final
# process exit code using following formula.
QEMU_SUCCESS_EXIT_CODE=$(((KERNEL_SUCCESS_EXIT_CODE << 1) | 1))

# Execute QEMU
# shellcheck disable=SC2086
${QEMU_ARGS} || exit_code=$?
exit_code=${exit_code:-0}

# Check if the execution was successful:
# - Exit code 0: Normal successful exit (e.g., ACPI shutdown or clean termination)
# - Exit code $QEMU_SUCCESS_EXIT_CODE: Kernel signaled success via isa-debug-exit device
if [ ${exit_code} -eq 0 ] || [ ${exit_code} -eq ${QEMU_SUCCESS_EXIT_CODE} ]; then
    exit 0
fi

exit ${exit_code}