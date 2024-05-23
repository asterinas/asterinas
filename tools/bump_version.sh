#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e

# This script is used to update Asterinas version numbers in all relevant files in the repository.
# Usage: ./tools/bump_version.sh <new_version>

# Update the package version (`version = "{version}"`) in file $1
update_package_version() {
    echo "Updating file $1"
    # Package version is usually the first version in Cargo.toml,
    # so only the first matched version is updated.
    pattern="^version = \"[[:digit:]]\+\.[[:digit:]]\+\.[[:digit:]]\+\"$"
    sed -i "0,/${pattern}/s/${pattern}/version = \"${new_version}\"/1" $1
}

# Update Docker image versions (`asterinas/asterinas:{version}`) in file $1
update_image_versions() {
    echo "Updating file $1"
    sed -i "s/asterinas\/asterinas:[[:digit:]]\+\.[[:digit:]]\+\.[[:digit:]]\+/asterinas\/asterinas:${new_version}/g" $1
}

SCRIPT_DIR=$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )
ASTER_SRC_DIR=${SCRIPT_DIR}/..
CARGO_TOML_PATH=${ASTER_SRC_DIR}/Cargo.toml
OSDK_CARGO_TOML_PATH=${ASTER_SRC_DIR}/osdk/Cargo.toml
VERSION_PATH=${ASTER_SRC_DIR}/VERSION

# Get and check the new version number
if [[ $1 =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    new_version=$1
else
    printf "Invalid version number: $1\nUsage: ./tools/bump_version.sh <new_version>\n"
    exit -1
fi

# Update the package version in Cargo.toml
update_package_version ${CARGO_TOML_PATH}
update_package_version ${OSDK_CARGO_TOML_PATH}

# Automatically bump Cargo.lock file
cargo update -p asterinas --precise $new_version

# Update Docker image versions in README files
update_image_versions ${ASTER_SRC_DIR}/README.md
update_image_versions ${ASTER_SRC_DIR}/README_CN.md
update_image_versions ${SCRIPT_DIR}/docker/README.md

# Update Docker image versions in workflows
WORKFLOWS=$(find "${ASTER_SRC_DIR}/.github/workflows/" -type f -name "*.yml")
for workflow in $WORKFLOWS; do
    update_image_versions $workflow
done

# Update Docker image versions in the documentation
GET_STARTED_PATH=${ASTER_SRC_DIR}/docs/src/kernel/README.md
update_image_versions $GET_STARTED_PATH

# Create or update VERSION
echo "${new_version}" > ${VERSION_PATH}

echo "Bumped Asterinas & OSDK version to $new_version"
