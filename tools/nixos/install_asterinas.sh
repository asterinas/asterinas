#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e

NIXOS_DIR=$(realpath $1)
SCRIPT_DIR=$(cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd)
ASTER_IMAGE_PATH=${NIXOS_DIR}/asterinas.img
BUILD_DIR=$(mktemp -d -p /mnt)
ASTERINAS_DIR=$(realpath ${SCRIPT_DIR}/../..)
DISTRO_DIR=$(realpath ${ASTERINAS_DIR}/distro)
NIXOS_DISK_SIZE_IN_MB=${NIXOS_DISK_SIZE_IN_MB:-"8196"}

mkdir -p ${NIXOS_DIR}
cp -rL ${ASTERINAS_DIR}/test/build/initramfs/etc/resolv.conf ${NIXOS_DIR}

export NIXOS_KERNEL=${NIXOS_KERNEL:-"$(realpath ${ASTERINAS_DIR}/target/osdk/iso_root/boot/aster-nix-osdk-bin)"}
export NIXOS_STAGE_1_INIT=${NIXOS_STAGE_1_INIT:-"$(realpath ${ASTERINAS_DIR}/tools/nixos/stage_1_init.sh)"}
export NIXOS_RESOLV_CONF=${NIXOS_RESOLV_CONF:-"$(realpath ${NIXOS_DIR}/resolv.conf)"}

echo "************  NIXOS SETTINGS  *************"
echo "DISK_SIZE: ${NIXOS_DISK_SIZE_IN_MB}MB"
echo "BUILD_DIR=${BUILD_DIR}"
echo "BUILD_IMAGE_PATH=${ASTER_IMAGE_PATH}"
echo "CONFIGURATION=${DISTRO_DIR}/configuration.nix"
echo "LOG_LEVEL=${LOG_LEVEL}"
echo "CONSOLE=${CONSOLE}"
echo "KERNEL=${NIXOS_KERNEL}"
echo "STAGE_1_INIT=${NIXOS_STAGE_1_INIT}"
echo "STAGE_2_INIT=${NIXOS_STAGE_2_INIT}"
echo "RESOLV_CONF=${NIXOS_RESOLV_CONF}"
echo "DISABLE_SYSTEMD=${NIXOS_DISABLE_SYSTEMD}"
echo "TEST_COMMAND=${NIXOS_TEST_COMMAND}"
echo "************END OF NIXOS SETTINGS************"

if [ ! -e ${ASTER_IMAGE_PATH} ]; then
    echo "Creating image at ${ASTER_IMAGE_PATH} of size ${NIXOS_DISK_SIZE_IN_MB}MB......"
    dd if=/dev/zero of=${ASTER_IMAGE_PATH} bs=1M count=${NIXOS_DISK_SIZE_IN_MB}
    echo "Image created successfully!"
fi

DEVICE=$(losetup -fP --show ${ASTER_IMAGE_PATH})
echo "${DEVICE} created"

if [ ! -b "${DEVICE}p1" ] && [ ! -b "${DEVICE}p2" ]; then
    parted ${DEVICE} -- mklabel gpt
    parted ${DEVICE} -- mkpart ESP fat32 1MB 512MB
    parted ${DEVICE} -- mkpart root ext2 512MB 100%
    parted ${DEVICE} -- set 1 esp on
    echo "partition finished"

    mkfs.fat -F 32 -n boot "${DEVICE}p1"
    mkfs.ext2 -L nixos "${DEVICE}p2"
    echo "mkfs finished"
else
    echo "Partitions ${DEVICE}p1 and ${DEVICE}p2 already exist — skipping partitioning and mkfs"
fi

if findmnt -M ${BUILD_DIR}/boot >/dev/null; then
	umount -d ${BUILD_DIR}/boot
fi
if findmnt -M ${BUILD_DIR} >/dev/null; then
	umount -d ${BUILD_DIR}
fi

mkdir -p ${BUILD_DIR}
mount -o sync,dirsync "${DEVICE}p2" ${BUILD_DIR}

mkdir -p ${BUILD_DIR}/boot
mkdir -p ${BUILD_DIR}/etc/nixos
mount -o umask=077,sync,dirsync "${DEVICE}p1" ${BUILD_DIR}/boot

echo "${BUILD_DIR} is mounted successfully!"

cleanup() {
    umount -d ${BUILD_DIR}/boot 2>/dev/null || true
    umount -d ${BUILD_DIR} 2>/dev/null || true
    losetup -d $DEVICE 2>/dev/null || true
    rm -rf ${BUILD_DIR}
}
trap cleanup EXIT INT TERM ERR

cp ${DISTRO_DIR}/configuration.nix ${BUILD_DIR}/etc/nixos
cp ${DISTRO_DIR}/aster_configuration.nix ${BUILD_DIR}/etc/nixos
cp -r ${DISTRO_DIR}/modules ${BUILD_DIR}/etc/nixos
cp -r ${DISTRO_DIR}/overlays ${BUILD_DIR}/etc/nixos

nixos-install --root ${BUILD_DIR} --no-root-passwd

echo "Congratulations! Asterinas NixOS has been installed successfully!"