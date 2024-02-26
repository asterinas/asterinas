#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

WORKSPACE_ROOT="$(dirname "$(readlink -f "$0")")/.."

EXCLUDED_CRATES=$(sed -n '/^\[workspace\]/,/^\[.*\]/{/exclude = \[/,/\]/p}' "$WORKSPACE_ROOT/Cargo.toml" | grep -v "exclude = \[" | tr -d '", \]')


CHECK_MODE=false

if [ "$#" -eq 1 ]; then
    if [ "$1" == "--check" ]; then
        CHECK_MODE=true
    else
        echo "Error: Invalid argument. Only '--check' is allowed."
        exit 1
    fi
elif [ "$#" -gt 1 ]; then
    echo "Error: Too many arguments. Only '--check' is allowed."
    exit 1
fi

cd $WORKSPACE_ROOT
if [ "$CHECK_MODE" = true ]; then
    cargo fmt --check
else
    cargo fmt
fi

for CRATE in $EXCLUDED_CRATES; do
    CRATE_DIR="$WORKSPACE_ROOT/$CRATE"

    # `cargo-component` crate currently is pinned to use Rust nightly-2023-02-05 version, 
    # and when using this script in the current Docker environment, it will 
    # additionally download this version of Rust. 
    # Here temporarily skip processing this crate for now considering that this crate 
    # is not currently in use or under development. 
    case "$CRATE" in
        *cargo-component*)
            continue
            ;;
    esac

    if [ -d "$CRATE_DIR" ]; then
        if [ "$CHECK_MODE" = true ]; then
            (cd "$CRATE_DIR" && cargo fmt --check)
        else
            (cd "$CRATE_DIR" && cargo fmt)
        fi
    else
        echo "Directory for crate $CRATE does not exist"
    fi
done
