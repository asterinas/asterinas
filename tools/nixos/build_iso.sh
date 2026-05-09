#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e

TARGET_ARCH=${TARGET_ARCH:-x86_64}
# Accept config file name as parameter, default to "configuration.nix"
CONFIG_FILE_NAME=${1:-"configuration.nix"}

SCRIPT_DIR=$(cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd)
ASTERINAS_DIR=$(realpath ${SCRIPT_DIR}/../..)
DISTRO_DIR=$(realpath ${ASTERINAS_DIR}/distro)
TARGET_DIR=${ASTERINAS_DIR}/target/nixos
VERSION=$(cat ${ASTERINAS_DIR}/VERSION)

NIX_SYSTEM=$("${SCRIPT_DIR}/print_target_nix_system.sh" "${TARGET_ARCH}") || exit 1

mkdir -p ${TARGET_DIR}

nix-build ${DISTRO_DIR}/iso_image \
    --argstr target_platform "${NIX_SYSTEM}" \
    --arg autoInstall ${AUTO_INSTALL} \
    --argstr config-file-name "${CONFIG_FILE_NAME}" \
    --argstr extra-substituters "${RELEASE_SUBSTITUTER} ${DEV_SUBSTITUTER}" \
    --argstr extra-trusted-public-keys "${RELEASE_TRUSTED_PUBLIC_KEY} ${DEV_TRUSTED_PUBLIC_KEY}" \
    --argstr version ${VERSION} \
    --out-link ${TARGET_DIR}/iso_image
