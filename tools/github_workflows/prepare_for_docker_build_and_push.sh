#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e

if [[ -z "$1" || -z "$2" ]]; then
    echo "Prepare the environment for the Github action of docker/build-push-action"
    echo "Usage: $0 <username> <token>"
    exit 1
fi

USERNAME="$1"
TOKEN="$2"

# Step 1: Login to Docker Hub
echo "Logging in to Docker Hub..."
echo "${TOKEN}" | docker login -u "${USERNAME}" --password-stdin || {
    echo "Docker login failed"
    exit 2
}

# Step 2: Fetch versions
echo "Fetching Docker image version and Rust version..."
ASTER_SRC_DIR=$(dirname "$0")/../..
IMAGE_VERSION=$(cat "${ASTER_SRC_DIR}/DOCKER_IMAGE_VERSION")
RUST_VERSION=$(grep -m1 -o 'nightly-[0-9]\+-[0-9]\+-[0-9]\+' "${ASTER_SRC_DIR}/rust-toolchain.toml")
echo "image_version=$IMAGE_VERSION" >> "${GITHUB_OUTPUT}"
echo "rust_version=$RUST_VERSION" >> "${GITHUB_OUTPUT}"

# Step 3: Check whether each target image already exists.
echo "Checking if Docker images exist..."
for image_name in osdk nix asterinas; do
    docker_image="asterinas/${image_name}:${IMAGE_VERSION}"
    if docker manifest inspect "${docker_image}" > /dev/null 2>&1; then
        echo "${image_name}_existed=true" >> "${GITHUB_OUTPUT}"
    else
        echo "${image_name}_existed=false" >> "${GITHUB_OUTPUT}"
    fi
done
