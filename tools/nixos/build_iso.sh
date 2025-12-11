#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e

SCRIPT_DIR=$(cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd)
ASTERINAS_DIR=$(realpath ${SCRIPT_DIR}/../..)
DISTRO_DIR=$(realpath ${ASTERINAS_DIR}/distro)
TARGET_DIR=${ASTERINAS_DIR}/target/nixos

mkdir -p ${TARGET_DIR}

nix-build ${DISTRO_DIR}/iso_image \
    --arg autoInstall ${AUTO_INSTALL} \
    --argstr test-command "${NIXOS_TEST_COMMAND}" \
    --out-link ${TARGET_DIR}/iso_image
