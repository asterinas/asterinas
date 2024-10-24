#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e

SCRIPT_DIR=$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )
ASTER_SRC_DIR=${SCRIPT_DIR}/../..
CARGO_TOML_PATH=${SCRIPT_DIR}/../../Cargo.toml
VERSION=$( cat ${ASTER_SRC_DIR}/VERSION )

if [ "$1" = "intel-tdx" ]; then
    IMAGE_NAME="asterinas/asterinas:${VERSION}-tdx"
else
    IMAGE_NAME="asterinas/asterinas:${VERSION}"
fi

docker run -it --privileged --network=host --device=/dev/kvm --device=/dev/vhost-net -v ${ASTER_SRC_DIR}:/root/asterinas ${IMAGE_NAME}
