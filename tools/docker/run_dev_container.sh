#!/bin/bash

set -e

SCRIPT_DIR=$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )
JINUX_SRC_DIR=${SCRIPT_DIR}/../..
CARGO_TOML_PATH=${SCRIPT_DIR}/../../Cargo.toml
VERSION=$( cat ${JINUX_SRC_DIR}/VERSION )
IMAGE_NAME=jinuxdev/jinux:${VERSION}

docker run -it --privileged --network=host --device=/dev/kvm -v ${JINUX_SRC_DIR}:/root/jinux ${IMAGE_NAME}
