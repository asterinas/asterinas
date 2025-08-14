#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e

if [[ -z "$1" || -z "$2" || -z "$3" ]]; then
    echo "Prepare the environment for the Github action of docker/build-push-action"
    echo "Usage: $0 <username> <token> <image_name>"
    exit 1
fi

USERNAME="$1"
TOKEN="$2"
IMAGE_NAME="$3"

# Step 1: Set up Docker Buildx
echo "Setting up Docker Buildx..."
docker buildx create --use || {
    echo "Failed to set up Docker Buildx"
    exit 1
}

# Step 2: Login to Docker Hub
echo "Logging in to Docker Hub..."
echo "${TOKEN}" | docker login -u "${USERNAME}" --password-stdin || {
    echo "Docker login failed"
    exit 2
}

# Step 3: Fetch versions
echo "Fetching Docker image version and Rust version..."
ASTER_SRC_DIR=$(dirname "$0")/../..
IMAGE_VERSION=$(cat ${ASTER_SRC_DIR}/DOCKER_IMAGE_VERSION)
RUST_VERSION=$(grep -m1 -o 'nightly-[0-9]\+-[0-9]\+-[0-9]\+' ${ASTER_SRC_DIR}/rust-toolchain.toml)
echo "image_version=$IMAGE_VERSION" >> $GITHUB_OUTPUT
echo "rust_version=$RUST_VERSION" >> $GITHUB_OUTPUT

# Step 4: Check if Docker image exists
echo "Checking if Docker image exists..."
if [[ "${IMAGE_NAME}" == "osdk" ]]; then
    DOCKER_IMAGE="asterinas/osdk:${IMAGE_VERSION}"
elif [[ "${IMAGE_NAME}" == "nix" ]]; then
    DOCKER_IMAGE="asterinas/nix:${IMAGE_VERSION}"
elif [[ "${IMAGE_NAME}" == "asterinas" ]]; then
    DOCKER_IMAGE="asterinas/asterinas:${IMAGE_VERSION}"
else
    echo "Error: Unknown image name '${IMAGE_NAME}'"
    exit 4
fi
if docker manifest inspect "${DOCKER_IMAGE}" > /dev/null 2>&1; then
    echo "is_existed=true" >> $GITHUB_OUTPUT
else
    echo "is_existed=false" >> $GITHUB_OUTPUT
fi
