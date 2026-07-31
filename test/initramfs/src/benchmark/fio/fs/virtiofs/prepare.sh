#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

mkdir -p "$FIO_MOUNT_POINT"

if mountpoint -q "$FIO_MOUNT_POINT"; then
    exit 0
fi

mount -t virtiofs "$VIRTIOFS_TAG" "$FIO_MOUNT_POINT"
