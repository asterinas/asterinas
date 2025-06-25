#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e

# This script is used to update Asterinas version numbers in all relevant files in the repository.
# Usage: ./tools/bump_version.sh command [options]
# Commands:
#   --docker_version_file [major|minor|patch|date]   Bump the Docker image version in the DOCKER_IMAGE_VERSION file under the project root
#   --docker_version_refs                            Update all references to the Docker image version throughout the codebase
#   --version_file                                   Bump the project version to match the Docker image version
#   --help, -h                                       Show this help message
# Options:
#   major, minor, patch, date                        The version part to increment when bumping the Docker image version

# TODO: we may remove the VERSION file in the future, 
# and retrieve the current version from git tag.

# Update the package version (`version = "{version}"`) in file $1
update_package_version() {
    echo "Updating file $1"
    # Package version is usually the first version in Cargo.toml,
    # so only the first matched version is updated.
    pattern="^version = \"[[:digit:]]\+\.[[:digit:]]\+\.[[:digit:]]\+\"$"
    sed -i "0,/${pattern}/s/${pattern}/version = \"${new_version}\"/1" $1
}

# Update the version of the $2 dependency (`$2 = { version = "", ...`) in file $1
update_dep_version() {
    echo "Updating the version of $2 in file $1"
    pattern="^$2 = { version = \"[[:digit:]]\+\.[[:digit:]]\+\.[[:digit:]]\+\""
    sed -i "0,/${pattern}/s/${pattern}/$2 = { version = \"${new_version}\"/1" $1
}

# Update Docker image versions (`asterinas/asterinas:{version}`) in file $1
update_image_versions() {
    echo "Updating file $1"
    # Update the version of the development container
    sed -i "s/asterinas\/asterinas:[[:digit:]]\+\.[[:digit:]]\+\.[[:digit:]]\+\(-[[:digit:]]\+\)\?/asterinas\/asterinas:${new_version}/g" $1
    # Update the test environment described in the OSDK manual
    sed -i "s/asterinas\/osdk:[[:digit:]]\+\.[[:digit:]]\+\.[[:digit:]]\+\(-[[:digit:]]\+\)\?/asterinas\/osdk:${new_version}/g" $1
}

# Print the help message
print_help() {
    echo "Usage: $0 <command> [options]"
    echo ""
    echo "Commands:"
    echo "  --docker_version_file [major|minor|patch|date]   Bump the Docker image version in the DOCKER_IMAGE_VERSION file under the project root"
    echo "  --docker_version_refs                            Update all references to the Docker image version throughout the codebase"
    echo "  --version_file                                   Bump the project version to match the Docker image version"
    echo "  --help, -h                                       Show this help message"
    echo ""
    echo "The [major|minor|patch|date] options for --docker_version_file specify which part of the"
    echo "Docker image version to increment. The 'date' option updates the date part of the version."
}

# Add the number $1 by 1
# Bash cannot deal with 0 by using `$((num + 1))`,
# So this function is defined to specially deal with 0.
add_one() {
    local num=$1
    if [ "$num" == "0" ]; then
        echo "1"
    else
        local bumped=$((num + 1))
        echo "$bumped"
    fi
}

# Update Docker image version in DOCKER_IMAGE_VERSION file
update_docker_image_version() {
    local IFS="-"
    local docker_version_parts=($(cat ${DOCKER_IMAGE_VERSION_PATH}))
    local version_part=$1

    if [[ -z "$version_part" ]]; then
        echo "Error: A version part (major, minor, patch, or date) must be specified."
        print_help
        exit 1
    fi

    case "$version_part" in
        "major" | "minor" | "patch")
            local IFS="."
            local semantic_version_parts=(${docker_version_parts[0]})
            if [ "$version_part" == "major" ]; then
                semantic_version_parts[0]=$(add_one "${semantic_version_parts[0]}")
                semantic_version_parts[1]=0
                semantic_version_parts[2]=0
            elif [ "$version_part" == "minor" ]; then
                semantic_version_parts[1]=$(add_one "${semantic_version_parts[1]}")
                semantic_version_parts[2]=0
            else # patch
                semantic_version_parts[2]=$(add_one "${semantic_version_parts[2]}")
            fi
            docker_version_parts[0]="${semantic_version_parts[*]}"
            docker_version_parts[1]=$(date +%Y%m%d)
            ;;
        "date")
            docker_version_parts[1]=$(date +%Y%m%d)
            ;;
        *)
            echo "Error: Invalid version part. Allowed values are: major, minor, patch, or date."
            print_help
            exit 1
            ;;
    esac

    local IFS="+"
    new_docker_version="${docker_version_parts[0]}-${docker_version_parts[1]}"
    echo -n "${new_docker_version}" > ${DOCKER_IMAGE_VERSION_PATH}
    echo "Bumped Docker image version to $new_docker_version"
}

# Update all Docker version references (except VERSION)
update_all_docker_version_refs() {
    new_version=$(cat ${DOCKER_IMAGE_VERSION_PATH})

    # Update Docker image versions in README files
    update_image_versions ${ASTER_SRC_DIR}/README.md
    update_image_versions ${ASTER_SRC_DIR}/README_CN.md
    update_image_versions ${ASTER_SRC_DIR}/README_JP.md
    update_image_versions ${SCRIPT_DIR}/docker/README.md
    update_image_versions ${DOCS_DIR}/src/kernel/intel_tdx.md

    # Update Docker image versions in workflows
    ALL_WORKFLOWS=$(find "${ASTER_SRC_DIR}/.github/workflows/" -type f -name "*.yml")
    EXCLUDED_WORKFLOWS=(
        "${ASTER_SRC_DIR}/.github/workflows/push_git_tag.yml"
        "${ASTER_SRC_DIR}/.github/workflows/check_licenses.yml"
        "${ASTER_SRC_DIR}/.github/workflows/publish_docker_images.yml"
    )

    for workflow in $ALL_WORKFLOWS; do
        if ! [[ " ${EXCLUDED_WORKFLOWS[*]} " =~ " ${workflow} " ]]; then
            update_image_versions "$workflow"
        fi
    done

    # Update Docker image versions in the documentation
    GET_STARTED_PATH=${ASTER_SRC_DIR}/docs/src/kernel/README.md
    update_image_versions $GET_STARTED_PATH
}

# Update project dependencies (Cargo.toml and Cargo.lock)
update_project_dependencies() {
    # Update the versions in Cargo.toml
    update_package_version ${OSTD_TEST_CARGO_TOML_PATH}
    update_package_version ${OSTD_MACROS_CARGO_TOML_PATH}
    update_package_version ${OSTD_CARGO_TOML_PATH}
    update_package_version ${LINUX_BOOT_PARAMS_CARGO_TOML_PATH}
    update_package_version ${LINUX_BZIMAGE_BUILDER_CARGO_TOML_PATH}
    update_package_version ${LINUX_BZIMAGE_SETUP_CARGO_TOML_PATH}
    update_package_version ${OSDK_CARGO_TOML_PATH}
    update_package_version ${OSDK_TEST_RUNNER_CARGO_TOML_PATH}
    update_package_version ${OSDK_FRAME_ALLOCATOR_CARGO_TOML_PATH}
    update_package_version ${OSDK_HEAP_ALLOCATOR_CARGO_TOML_PATH}

    update_dep_version ${OSDK_TEST_RUNNER_CARGO_TOML_PATH} ostd
    update_dep_version ${OSDK_FRAME_ALLOCATOR_CARGO_TOML_PATH} ostd
    update_dep_version ${OSDK_HEAP_ALLOCATOR_CARGO_TOML_PATH} ostd
    update_dep_version ${OSTD_CARGO_TOML_PATH} ostd-test
    update_dep_version ${OSTD_CARGO_TOML_PATH} linux-boot-params
    update_dep_version ${OSTD_CARGO_TOML_PATH} ostd-macros
    update_dep_version ${LINUX_BZIMAGE_SETUP_CARGO_TOML_PATH} linux-boot-params
    update_dep_version ${OSDK_CARGO_TOML_PATH} linux-bzimage-builder

    # Automatically bump Cargo.lock files
    cargo update -p aster-nix --precise $new_version # For Cargo.lock
    cd ${OSDK_DIR} && cargo update -p cargo-osdk --precise $new_version # For osdk/Cargo.lock
}

# Synchronize project version to Docker version (update VERSION)
sync_project_version() {
    new_version=$(cat ${DOCKER_IMAGE_VERSION_PATH} | cut -d'-' -f1)
    current_version=$(cat ${VERSION_PATH})
    if [ -z "$new_version" ] || [ -z "$current_version" ]; then
        echo "Error: Version string is empty."
        exit 1
    fi

    # Check if versions are equal
    if [ "$new_version" = "$current_version" ]; then
        echo "Versions are equal. No action needed."
        exit 0
    fi

    # Compare semantic versions
    lower_version=$(printf '%s\n' "$new_version" "$current_version" | sort -V | head -n1)
    if [ "$lower_version" = "$new_version" ]; then
        echo "Error: New version ($new_version) must be greater than current version ($current_version)."
        exit 1
    fi

    update_project_dependencies

    # Update tag version in release_tag workflow
    RELEASE_TAG_WORKFLOW=${ASTER_SRC_DIR}/.github/workflows/push_git_tag.yml
    update_tag_version $RELEASE_TAG_WORKFLOW

    echo -n "${new_version}" > ${VERSION_PATH}
    echo "Bumped Asterinas OSTD & OSDK version to $new_version"
}


# Update tag version (`v{version}`) in file $1
update_tag_version() {
    echo "Updating file $1"
    sed -i "s/v[[:digit:]]\+\.[[:digit:]]\+\.[[:digit:]]\+/v${new_version}/g" $1
}

SCRIPT_DIR=$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )
ASTER_SRC_DIR=${SCRIPT_DIR}/..
DOCS_DIR=${ASTER_SRC_DIR}/docs
OSTD_CARGO_TOML_PATH=${ASTER_SRC_DIR}/ostd/Cargo.toml
OSTD_TEST_CARGO_TOML_PATH=${ASTER_SRC_DIR}/ostd/libs/ostd-test/Cargo.toml
OSTD_MACROS_CARGO_TOML_PATH=${ASTER_SRC_DIR}/ostd/libs/ostd-macros/Cargo.toml
LINUX_BOOT_PARAMS_CARGO_TOML_PATH=${ASTER_SRC_DIR}/ostd/libs/linux-bzimage/boot-params/Cargo.toml
LINUX_BZIMAGE_BUILDER_CARGO_TOML_PATH=${ASTER_SRC_DIR}/ostd/libs/linux-bzimage/builder/Cargo.toml
LINUX_BZIMAGE_SETUP_CARGO_TOML_PATH=${ASTER_SRC_DIR}/ostd/libs/linux-bzimage/setup/Cargo.toml
OSDK_DIR=${ASTER_SRC_DIR}/osdk
OSDK_CARGO_TOML_PATH=${OSDK_DIR}/Cargo.toml
OSDK_TEST_RUNNER_CARGO_TOML_PATH=${ASTER_SRC_DIR}/osdk/deps/test-kernel/Cargo.toml
OSDK_FRAME_ALLOCATOR_CARGO_TOML_PATH=${ASTER_SRC_DIR}/osdk/deps/frame-allocator/Cargo.toml
OSDK_HEAP_ALLOCATOR_CARGO_TOML_PATH=${ASTER_SRC_DIR}/osdk/deps/heap-allocator/Cargo.toml
VERSION_PATH=${ASTER_SRC_DIR}/VERSION
DOCKER_IMAGE_VERSION_PATH=${ASTER_SRC_DIR}/DOCKER_IMAGE_VERSION

command=$1

if [[ "$command" == "--help" || "$command" == "-h" ]]; then
  print_help
  exit 0
fi

case "$command" in
    "--docker_version_file")
        update_docker_image_version "$2"
        ;;
    "--docker_version_refs")
        update_all_docker_version_refs
        ;;
    "--version_file")
        sync_project_version
        ;;
    *)
        echo "Warning: Using --docker_version_file, --docker_version_refs, or --version_file instead."
        ;;
esac
