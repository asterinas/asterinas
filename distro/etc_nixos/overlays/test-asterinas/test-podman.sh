#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

CONTAINER_NAME=c1
IMAGE_NAME=docker.io/library/alpine

podman run --name=${CONTAINER_NAME} ${IMAGE_NAME} ls /etc \
    | grep -q "^alpine-release" \
    || (echo "Test 'podman run' failed" && exit 1)
podman image ls \
    | grep -q ${IMAGE_NAME} \
    || (echo "Test 'podman image ls' failed" && exit 1)
podman ps -a \
    | grep -q "Exited (0)" \
    || (echo "Test 'podman ps -a' failed" && exit 1)
podman rm ${CONTAINER_NAME} || (echo "Test 'podman rm' failed" && exit 1)

echo "Test podman succeeds"