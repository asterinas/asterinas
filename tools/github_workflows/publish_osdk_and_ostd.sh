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

# Tracks packages that have already passed `cargo publish --dry-run` in this
# script. `--dry-run` does not really publish earlier packages to `crates.io`,
# so later packages patch only these already-checked local `path` dependencies
# to mirror the dependency state that a real publish would see.
declare -A DRY_RUN_PUBLISHED_PACKAGES=()

mark_dry_run_published() {
    DRY_RUN_PUBLISHED_PACKAGES[$1]=true
}

is_dry_run_published() {
    [ -n "${DRY_RUN_PUBLISHED_PACKAGES[$1]}" ]
}

# Performs the publish check or publishes the crate in directory $1.
# All arguments after $1 are passed to `cargo publish`.
do_publish_for() {
    pushd "$ASTER_SRC_DIR/$1"
    shift

    local ADDITIONAL_ARGS=("$@")
    local RF="$RUSTFLAGS --check-cfg cfg(ktest)"

    if [ -n "$DRY_RUN" ]; then
        local DEP_NAME DEP_PATH LOCAL_PATH_DEPS MANIFEST_PATH METADATA PACKAGE_METADATA PACKAGE_NAME
        local PATCH_ARGS=()

        MANIFEST_PATH="$(pwd -P)/Cargo.toml"
        # `cargo metadata` prints a JSON document for the **whole workspace**,
        # not only for `MANIFEST_PATH`. Each package entry includes fields such as
        # `manifest_path`, `name`, and `dependencies`; the `--dry-run` logic below
        # uses them to find the package being published and its local `path` deps.
        METADATA=$(cargo metadata --format-version=1 --no-deps --manifest-path "$MANIFEST_PATH")

        # Pick the package entry for the manifest currently being published.
        PACKAGE_METADATA=$(jq --arg manifest "$MANIFEST_PATH" '
            .packages[]
            | select(.manifest_path == $manifest)
        ' <<< "$METADATA")
        if [ -z "$PACKAGE_METADATA" ]; then
            echo "Error: cargo metadata did not contain a package for manifest: $MANIFEST_PATH" >&2
            exit 1
        fi

        PACKAGE_NAME=$(jq -r '.name' <<< "$PACKAGE_METADATA")
        if [ -z "$PACKAGE_NAME" ]; then
            echo "Error: cargo metadata did not contain a package name for manifest: $MANIFEST_PATH" >&2
            exit 1
        fi

        # List local non-dev `path` dependencies as tab-separated
        # `name<TAB>path` rows.
        LOCAL_PATH_DEPS=$(jq -r '
            .dependencies[]
            | select(.path != null and .source == null and .kind != "dev")
            | [.name, .path]
            | @tsv
        ' <<< "$PACKAGE_METADATA")

        if [ -n "$LOCAL_PATH_DEPS" ]; then
            while IFS=$'\t' read -r DEP_NAME DEP_PATH; do
                if is_dry_run_published "$DEP_NAME"; then
                    PATCH_ARGS+=(--config "patch.crates-io.$DEP_NAME.path=\"$DEP_PATH\"")
                fi
            done <<< "$LOCAL_PATH_DEPS"
        fi

        # Perform checks
        RUSTFLAGS=$RF cargo publish --dry-run --allow-dirty "${PATCH_ARGS[@]}" "${ADDITIONAL_ARGS[@]}"
        RUSTFLAGS=$RF cargo doc "${ADDITIONAL_ARGS[@]}"

        mark_dry_run_published "$PACKAGE_NAME"
    else
        RUSTFLAGS=$RF cargo publish --token $TOKEN "${ADDITIONAL_ARGS[@]}"
    fi

    popd
}

# The order of `do_publish_for` calls matters:
# `do_publish_for path/to/crate-a`
# must appear before
# `do_publish_for path/to/crate-b`
# if `crate-a` is a dependency of `crate-b`.

do_publish_for ostd/libs/linux-bzimage/boot-params
do_publish_for ostd/libs/linux-bzimage/builder
do_publish_for ostd/libs/linux-bzimage/setup \
    --target ../builder/src/x86_64-i386_pm-none.json \
    -Zbuild-std=core,alloc,compiler_builtins \
    -Zjson-target-spec
do_publish_for osdk

# All supported targets of OSTD, this array should keep consistent with
# `package.metadata.docs.rs.targets` in `ostd/Cargo.toml`.
TARGETS="x86_64-unknown-none"
for TARGET in $TARGETS; do
    do_publish_for ostd/libs/ostd-macros --target $TARGET
    do_publish_for ostd/libs/ostd-test --target $TARGET
    do_publish_for ostd --target $TARGET
    do_publish_for osdk/deps/frame-allocator --target $TARGET
    do_publish_for osdk/deps/heap-allocator --target $TARGET
    do_publish_for osdk/deps/test-kernel --target $TARGET

    # For actual publishing, we should only publish once. Using any target that
    # OSTD supports is OK. Here we use the first target in the list.
    if [ -z "$DRY_RUN" ]; then
        break
    fi
done
