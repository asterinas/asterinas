#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

NIXOS_DIR=$(realpath $1)
SCRIPT_DIR=$(cd "$( dirname "$0" )" && pwd)
ASTER_IMAGE_PATH=${NIXOS_DIR}/asterinas.img
BUILD_DIR=$(mktemp -d -p /mnt)
ASTERINAS_DIR=$(realpath ${SCRIPT_DIR}/../..)
DISTRO_DIR=${DISTRO_DIR:-"$(realpath ${ASTERINAS_DIR}/distro)"}
NIXOS_DISK_SIZE_IN_MB=${NIXOS_DISK_SIZE_IN_MB:-"8196"}

mkdir -p ${NIXOS_DIR}

if [ -n "$2" ]; then
    DEVICE=$(realpath $2)
    if [ ! -e "${DEVICE}" ] || [ ! -b "${DEVICE}" ]; then
        echo "Error: ${DEVICE} does not exist or is not a block device"
        exit 1
    fi

    BOOT_DEVICE=${DEVICE}1
    ROOT_DEVICE=${DEVICE}2
else
    if [ ! -e ${ASTER_IMAGE_PATH} ]; then
        echo "Creating image at ${ASTER_IMAGE_PATH} of size ${NIXOS_DISK_SIZE_IN_MB}MB......"
        dd if=/dev/zero of=${ASTER_IMAGE_PATH} bs=1M count=${NIXOS_DISK_SIZE_IN_MB}
        echo "Image created successfully!"
    fi

    DEVICE=$(losetup -fP --show ${ASTER_IMAGE_PATH})
    echo "${DEVICE} created"
    BOOT_DEVICE=${DEVICE}p1
    ROOT_DEVICE=${DEVICE}p2
fi

export NIXOS_KERNEL=${NIXOS_KERNEL:-"$(realpath ${ASTERINAS_DIR}/target/osdk/iso_root/boot/aster-nix-osdk-bin)"}
export NIXOS_STAGE_1_INIT=${NIXOS_STAGE_1_INIT:-"$(realpath ${ASTERINAS_DIR}/tools/nixos/stage_1_init.sh)"}

echo "************  NIXOS SETTINGS  *************"
echo "DISK_SIZE: ${NIXOS_DISK_SIZE_IN_MB}MB"
echo "BUILD_DIR=${BUILD_DIR}"
echo "BOOT_DEVICE=${BOOT_DEVICE}"
echo "ROOT_DEVICE=${ROOT_DEVICE}"
echo "CONFIGURATION=${DISTRO_DIR}/configuration.nix"
echo "LOG_LEVEL=${LOG_LEVEL}"
echo "CONSOLE=${CONSOLE}"
echo "KERNEL=${NIXOS_KERNEL}"
echo "STAGE_1_INIT=${NIXOS_STAGE_1_INIT}"
echo "STAGE_2_INIT=${NIXOS_STAGE_2_INIT}"
echo "RESOLV_CONF=${NIXOS_RESOLV_CONF}"
echo "DISABLE_SYSTEMD=${NIXOS_DISABLE_SYSTEMD}"
echo "************END OF NIXOS SETTINGS************"

if [ ! -b "${BOOT_DEVICE}" ] && [ ! -b "${ROOT_DEVICE}" ]; then
    parted -s ${DEVICE} -- mklabel gpt
    parted -s ${DEVICE} -- mkpart ESP fat32 1MB 512MB
    parted -s ${DEVICE} -- mkpart root ext2 512MB 100%
    parted -s ${DEVICE} -- set 1 esp on
    echo "partition finished"
    sync

    mkfs.fat -F 32 -n boot "${BOOT_DEVICE}"
    mkfs.ext2 -L nixos "${ROOT_DEVICE}"
    echo "mkfs finished"
else
    echo "Partitions ${BOOT_DEVICE} and ${ROOT_DEVICE} already exist — skipping partitioning and mkfs"
fi

if findmnt -M ${BUILD_DIR}/boot >/dev/null; then
	umount -d ${BUILD_DIR}/boot
fi
if findmnt -M ${BUILD_DIR} >/dev/null; then
	umount -d ${BUILD_DIR}
fi

mkdir -p ${BUILD_DIR}
mount "${ROOT_DEVICE}" ${BUILD_DIR}

mkdir -p ${BUILD_DIR}/boot
mkdir -p ${BUILD_DIR}/etc/nixos
mount -o umask=077 "${BOOT_DEVICE}" ${BUILD_DIR}/boot

echo "${BUILD_DIR} is mounted successfully!"

cleanup() {
    umount -d ${BUILD_DIR}/boot 2>/dev/null || true
    umount -d ${BUILD_DIR} 2>/dev/null || true
    if [ -z "$2" ]; then
        losetup -d $DEVICE 2>/dev/null || true
    fi
    rm -rf ${BUILD_DIR}
}
trap cleanup EXIT INT TERM

cp ${DISTRO_DIR}/configuration.nix ${BUILD_DIR}/etc/nixos
cp ${DISTRO_DIR}/aster_configuration.nix ${BUILD_DIR}/etc/nixos
cp -r ${DISTRO_DIR}/modules ${BUILD_DIR}/etc/nixos
cp -r ${DISTRO_DIR}/overlays ${BUILD_DIR}/etc/nixos

nixos-install --root ${BUILD_DIR} --no-root-passwd

echo "Congratulations! Asterinas NixOS has been installed successfully!"