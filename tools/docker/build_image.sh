#!/bin/bash

set -e

SCRIPT_DIR=$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )
CARGO_TOML_PATH=${SCRIPT_DIR}/../../Cargo.toml
VERSION=$( grep -m1 -o '[0-9]\+\.[0-9]\+\.[0-9]\+' ${CARGO_TOML_PATH} | sed 's/[^0-9\.]//g'  )
IMAGE_NAME=jinuxdev/jinux:${VERSION}
DOCKER_FILE=${SCRIPT_DIR}/Dockerfile.ubuntu22.04
ARCH=linux/amd64

# Build docker
cd ${SCRIPT_DIR}
docker buildx build -f ${DOCKER_FILE} --platform ${ARCH} -t ${IMAGE_NAME} .
