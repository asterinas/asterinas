#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

NEW_ROOT=""
NEW_INIT=""
BREAK=""
ARGS=""

for arg in "$@"; do
    case "$arg" in
        root=*)
            NEW_ROOT="${arg#root=}"
            ;;
        init=*)
            NEW_INIT="${arg#init=}"
            ;;
        rd.break=*)
            BREAK="${arg#rd.break=}"
            ;;
        *)
            ARGS="$ARGS $arg"
            ;;
    esac
done

if [ "$BREAK" = "1" ]; then
    echo "Breaking into initramfs shell..."
    exec /bin/sh
fi

if [ -z "$NEW_ROOT" ] || [ -z "$NEW_INIT" ]; then
    echo "Error: 'root=' and 'init=' parameters are required."
    exit 1
fi

mkdir /sysroot
mount -t ext2 $NEW_ROOT /sysroot
mount -t proc none /sysroot/proc
mount --move /dev /sysroot/dev

exec switch_root /sysroot $NEW_INIT $ARGS