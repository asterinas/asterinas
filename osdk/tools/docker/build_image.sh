#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e

SCRIPT_DIR=$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )
ASTER_ROOT_DIR=${SCRIPT_DIR}/../../..
ASTER_RUST_VERSION=$( grep -m1 -o 'nightly-[0-9]\+-[0-9]\+-[0-9]\+' ${ASTER_ROOT_DIR}/rust-toolchain.toml )
VERSION=$( cat ${ASTER_ROOT_DIR}/VERSION )
DOCKERFILE=${SCRIPT_DIR}/Dockerfile

if [ "$1" = "intel-tdx" ]; then
    IMAGE_NAME="asterinas/osdk:${VERSION}-tdx"
    python3 gen_dockerfile.py --intel-tdx
else
    IMAGE_NAME="asterinas/osdk:${VERSION}"
    python3 gen_dockerfile.py
fi

docker build \
    -t ${IMAGE_NAME} \
    --build-arg ASTER_RUST_VERSION=${ASTER_RUST_VERSION} \
    -f ${DOCKERFILE} \
    ${SCRIPT_DIR} 
