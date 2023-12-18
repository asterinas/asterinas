#!/bin/bash

# This script is used to update Jinux version numbers in all relevant files in the repository.
# Usage: ./tools/bump_version.sh <new_version>

# Update Cargo style versions (`version = "{version}"`) in file $1
update_cargo_versions() {
    echo "Updating file $1"
    sed -i "s/^version = \"[[:digit:]]\+\.[[:digit:]]\+\.[[:digit:]]\+\"$/version = \"${new_version}\"/g" $1
}

# Update Docker image versions (`jinuxdev/jinux:{version}`) in file $1
update_image_versions() {
    echo "Updating file $1"
    sed -i "s/jinuxdev\/jinux:[[:digit:]]\+\.[[:digit:]]\+\.[[:digit:]]\+/jinuxdev\/jinux:${new_version}/g" $1
}

SCRIPT_DIR=$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )
JINUX_SRC_DIR=${SCRIPT_DIR}/..
CARGO_TOML_PATH=${JINUX_SRC_DIR}/Cargo.toml
VERSION_PATH=${JINUX_SRC_DIR}/VERSION

# Get and check the new version number
if [[ $1 =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    new_version=$1
else
    printf "Invalid version number: $1\nUsage: ./tools/bump_version.sh <new_version>\n"
    exit -1
fi

# Update Cargo.toml
update_cargo_versions ${CARGO_TOML_PATH}

# Automatically bump Cargo.lock file
cargo update -p jinux --precise $new_version

# Update Docker image versions in README files
update_image_versions ${JINUX_SRC_DIR}/README.md
update_image_versions ${SCRIPT_DIR}/docker/README.md

# Update Docker image versions in workflows
WORKFLOWS=$(find "${JINUX_SRC_DIR}/.github/workflows/" -type f -name "*.yml")
for workflow in $WORKFLOWS; do
    update_image_versions $workflow
done

# Create or update VERSION
echo "${new_version}" > ${VERSION_PATH}

echo "Bumped Jinux version to $new_version"
