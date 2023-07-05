#!/bin/bash

set -e

SCRIPT_DIR=$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )
JINUX_SRC_DIR=${SCRIPT_DIR}/../..
CARGO_TOML_PATH=${SCRIPT_DIR}/../../Cargo.toml
VERSION=$( grep -m1 -o '[0-9]\+\.[0-9]\+\.[0-9]\+' ${CARGO_TOML_PATH} | sed 's/[^0-9\.]//g'  )
IMAGE_NAME=jinuxdev/jinux:${VERSION}

docker run -it --privileged --network=host --device=/dev/kvm -v ${JINUX_SRC_DIR}:/root/jinux ${IMAGE_NAME}
