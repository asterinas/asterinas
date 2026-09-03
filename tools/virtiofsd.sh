#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -euo pipefail

if [[ "${VIRTIOFS:-off}" != "on" ]]; then
    exit 0
fi

socket=${VIRTIOFS_SOCKET:-"${VIRTIOFS_RUNTIME_DIR:-"$PWD/.osdk-virtiofs"}/vfs.sock"}
shared_dir=${VIRTIOFS_SHARED_DIR:-"${VIRTIOFS_RUNTIME_DIR:-"$PWD/.osdk-virtiofs"}/shared"}
log_file=${VIRTIOFS_LOG_FILE:-"${VIRTIOFS_RUNTIME_DIR:-"$PWD/.osdk-virtiofs"}/virtiofsd.log"}

mkdir -p "$(dirname "$socket")" "$shared_dir"
rm -f "$socket"

exec "${VIRTIOFSD:-/usr/libexec/virtiofsd}" \
    --shared-dir "$shared_dir" \
    --socket-path "$socket" \
    --cache "${VIRTIOFS_CACHE:-auto}" \
    >>"$log_file" 2>&1
