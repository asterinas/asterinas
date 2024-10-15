#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e

# Publish OSDK and OSTD to crates.io
#
# Usage: publish_osdk_and_ostd.sh [--dry-run | --token REGISTRY_TOKEN]

SCRIPT_DIR=$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )
ASTER_SRC_DIR=${SCRIPT_DIR}/../..

# Print help message
print_help() {
    echo "Usage: $0 [--dry-run | --token REGISTRY_TOKEN]"
    echo ""
    echo "Options:"
    echo "  --dry-run:               Execute the publish check without actually publishing it."
    echo "  --token REGISTRY_TOKEN:  The token to authenticate with crates.io."
}

# Parse the parameters
DRY_RUN=""
TOKEN=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --dry-run)
            DRY_RUN="true"
            shift
            ;;
        --token)
            TOKEN="$2"
            shift 2
            ;;
        *)
            echo "Error: Invalid parameter: $1"
            print_help
            exit 1
            ;;
    esac
done

# Performs the publish check or publish the crate in directory $1, with
# optional target $2. If the target is not specified, cargo will decide
# the target automatically.
do_publish_for() {
    pushd $ASTER_SRC_DIR/$1
    TARGET_ARGS=""
    if [ -n "$2" ]; then
        TARGET_ARGS="--target $2"
    fi
    if [ -n "$DRY_RUN" ]; then
        cargo publish --dry-run $TARGET_ARGS
        cargo doc $TARGET_ARGS
    else
        cargo publish --token $TOKEN $TARGET_ARGS
    fi
    popd
}

do_publish_for osdk
do_publish_for ostd/libs/linux-bzimage/build
do_publish_for ostd/libs/linux-bzimage/boot-params

# All supported targets of OSTD, this array should keep consistent with
# `package.metadata.docs.rs.targets` in `ostd/Cargo.toml`.
TARGETS="x86_64-unknown-none"
for TARGET in $TARGETS; do
    do_publish_for ostd/libs/ostd-macros $TARGET
    do_publish_for ostd/libs/ostd-test $TARGET
    do_publish_for ostd/libs/linux-bzimage/setup $TARGET
    do_publish_for ostd $TARGET
    do_publish_for osdk/test-kernel $TARGET

    # For actual publishing, we should only publish once. Using any target that
    # OSTD supports is OK. Here we use the first target in the list.
    if [ -z "$DRY_RUN" ]; then
        break
    fi
done
