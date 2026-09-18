#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -euo pipefail

virtiofsd=/usr/libexec/virtiofsd
runtime_dir=/tmp/asterinas-virtiofs
cache=auto

usage() {
    cat <<'EOF'
Usage: run_virtiofsd.sh [OPTIONS]

Run virtiofsd with paths rooted under the runtime directory.

Options:
  --path PATH         virtiofsd executable (default: /usr/libexec/virtiofsd)
  --runtime-dir PATH  Runtime directory (default: /tmp/asterinas-virtiofs)
  --cache MODE        virtiofsd cache mode (auto, always, never, metadata; default: auto)
  -h, --help          Show this help message
EOF
}

while (($# > 0)); do
    case "$1" in
        --path|--runtime-dir|--cache)
            [[ $# -ge 2 ]] || { echo "missing argument for $1" >&2; exit 2; }
            case "$1" in
                --path) virtiofsd=$2 ;;
                --runtime-dir) runtime_dir=$2 ;;
                --cache) cache=$2 ;;
            esac
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "unknown option: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

socket=$runtime_dir/vfs.sock
shared_dir=$runtime_dir/shared
log_file=$runtime_dir/virtiofsd.log

mkdir -p "$runtime_dir" "$shared_dir"
rm -f "$socket"

exec "$virtiofsd" \
    --shared-dir "$shared_dir" \
    --socket-path "$socket" \
    --cache "$cache" \
    >>"$log_file" 2>&1
