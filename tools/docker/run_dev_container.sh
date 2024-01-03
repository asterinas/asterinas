#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e

SCRIPT_DIR=$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )
ASTER_SRC_DIR=${SCRIPT_DIR}/../..
CARGO_TOML_PATH=${SCRIPT_DIR}/../../Cargo.toml
VERSION=$( cat ${ASTER_SRC_DIR}/VERSION )
IMAGE_NAME=asterinas/asterinas:${VERSION}

docker run -it --privileged --network=host --device=/dev/kvm -v ${ASTER_SRC_DIR}:/root/asterinas ${IMAGE_NAME}
