#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

if [ ! -d "$FIO_MOUNT_POINT" ]; then
    echo "Expected fio mount point $FIO_MOUNT_POINT to exist" >&2
    exit 1
fi
