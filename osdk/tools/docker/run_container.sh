#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e

SCRIPT_DIR=$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )
ASTER_ROOT_DIR=${SCRIPT_DIR}/../..
VERSION=$( cat ${ASTER_ROOT_DIR}/VERSION )
IMAGE_NAME="asterinas/osdk:${VERSION}"

docker run -it -v ${ASTER_ROOT_DIR}:/root/asterinas ${IMAGE_NAME}
