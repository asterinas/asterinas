#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

# A failed file search must fail the check even if nixfmt succeeds.
set -euo pipefail

options=()
if [[ "${1:-}" == "--check" ]]; then
    options+=(--check)
    shift
fi

if [[ "$#" -eq 0 ]]; then
    echo "Usage: $0 [--check] <file-or-directory>..." >&2
    exit 2
fi

if ! command -v nixfmt >/dev/null; then
    echo "nixfmt is not in PATH" >&2
    exit 1
fi

roots=()
for path in "$@"; do
    if [[ -z "$path" ]]; then
        echo "formatting paths must not be empty" >&2
        exit 2
    fi
    # Prefix relative paths so find does not interpret them as expressions.
    case "$path" in
        /*) roots+=("$path") ;;
        *) roots+=("./$path") ;;
    esac
done

find "${roots[@]}" -type f -name '*.nix' -print0 |
    xargs -0 -r -n 1 nixfmt "${options[@]}" --
