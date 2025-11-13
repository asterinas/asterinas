#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e

# Publish sctrace to crates.io
#
# Usage: publish_sctrace.sh [--dry-run | --token REGISTRY_TOKEN]

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

# Performs the publish check or publish the crate in directory $1.
# All the arguments after $1 are passed to `cargo publish`.
do_publish_for() {
    pushd $ASTER_SRC_DIR/$1

    ADDITIONAL_ARGS="${@:2}"
    RF="$RUSTFLAGS --check-cfg cfg(ktest)"

    if [ -n "$DRY_RUN" ]; then
        # Perform checks
        RUSTFLAGS=$RF cargo publish --dry-run --allow-dirty $ADDITIONAL_ARGS
        RUSTFLAGS=$RF cargo doc $ADDITIONAL_ARGS
    else
        RUSTFLAGS=$RF cargo publish --token $TOKEN $ADDITIONAL_ARGS
    fi

    popd
}

do_publish_for tools/sctrace