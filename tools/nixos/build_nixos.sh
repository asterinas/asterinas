#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e

SCRIPT_DIR=$(cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd)
ASTERINAS_DIR=$(realpath ${SCRIPT_DIR}/../..)
ASTER_IMAGE_PATH=${ASTERINAS_DIR}/target/nixos/asterinas.img
DISTRO_DIR=$(realpath ${ASTERINAS_DIR}/distro)
# Accept config file name as parameter, default to "configuration.nix"
CONFIG_FILE_NAME=${1:-"configuration.nix"}
CONFIG_PATH=${DISTRO_DIR}/etc_nixos/${CONFIG_FILE_NAME}

pushd $DISTRO_DIR
nix-build aster_nixos_installer/default.nix \
    --argstr disable-systemd "${NIXOS_DISABLE_SYSTEMD}" \
    --argstr stage-2-hook "${NIXOS_STAGE_2_INIT}" \
    --argstr log-level "${LOG_LEVEL}" \
    --argstr console "${CONSOLE}" \
    --argstr extra-substituters "${RELEASE_SUBSTITUTER} ${DEV_SUBSTITUTER}" \
    --argstr extra-trusted-public-keys "${RELEASE_TRUSTED_PUBLIC_KEY} ${DEV_TRUSTED_PUBLIC_KEY}"
popd

mkdir -p ${ASTERINAS_DIR}/target/nixos
if [ ! -e ${ASTER_IMAGE_PATH} ]; then
    echo "Creating image at ${ASTER_IMAGE_PATH} of size ${NIXOS_DISK_SIZE_IN_MB}MB......"
    dd if=/dev/zero of=${ASTER_IMAGE_PATH} bs=1M count=${NIXOS_DISK_SIZE_IN_MB}
    echo "Image created successfully!"
fi

DISK=$(losetup -fP --show ${ASTER_IMAGE_PATH})
cleanup() {
    losetup -d ${DISK} 2>/dev/null || true
}
trap cleanup EXIT INT TERM ERR

${DISTRO_DIR}/result/bin/install_aster_nixos.sh --config ${CONFIG_PATH} --disk ${DISK}
