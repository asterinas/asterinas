#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

TARGET_ARCH=${1:-${TARGET_ARCH:-x86_64}}

case "${TARGET_ARCH}" in
    x86_64)
        echo "x86_64-linux"
        ;;
    aarch64)
        echo "aarch64-linux"
        ;;
    riscv64)
        echo "riscv64-linux"
        ;;
    loongarch64)
        echo "loongarch64-linux"
        ;;
    *)
        echo "Error: unsupported TARGET_ARCH=${TARGET_ARCH}" >&2
        exit 1
        ;;
esac
