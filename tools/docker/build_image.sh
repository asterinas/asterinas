#!/bin/bash

set -e

SCRIPT_DIR=$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )
CARGO_TOML_PATH=${SCRIPT_DIR}/../../Cargo.toml
VERSION=$( grep -m1 -o '[0-9]\+\.[0-9]\+\.[0-9]\+' ${CARGO_TOML_PATH} | sed 's/[^0-9\.]//g'  )
IMAGE_NAME=jinuxdev/jinux:${VERSION}
DOCKER_FILE=${SCRIPT_DIR}/Dockerfile.ubuntu22.04
BOM_DIR=${SCRIPT_DIR}/bom
TOP_DIR=${SCRIPT_DIR}/../../
ARCH=linux/amd64
RUST_TOOLCHAIN_PATH=${SCRIPT_DIR}/../../rust-toolchain.toml
JINUX_RUST_VERSION=$( grep -m1 -o 'nightly-[0-9]\+-[0-9]\+-[0-9]\+' ${RUST_TOOLCHAIN_PATH} )

# Prpare the BOM (bill of materials) directory to copy files or dirs into the docker image.
# This is because the `docker build` can not access the parent directory of the context.
if [ ! -d ${BOM_DIR} ]; then
    mkdir -p ${BOM_DIR}
    cp -rf ${TOP_DIR}/regression/syscall_test ${BOM_DIR}/
fi

# Build docker
cd ${SCRIPT_DIR}
docker buildx build -f ${DOCKER_FILE} \
    --build-arg JINUX_RUST_VERSION=${JINUX_RUST_VERSION} \
    --platform ${ARCH} \
    -t ${IMAGE_NAME} .
