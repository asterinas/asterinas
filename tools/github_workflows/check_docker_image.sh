#!/bin/bash
# SPDX-License-Identifier: MPL-2.0

# This runs inside a disposable candidate container, without network access.
set -euxo pipefail
export LC_ALL=C
qemu=$1
qemu_version=$2
vdso=$3
image=$4
gc=$5
nixpkgs_url=$6

test "$(readlink -f /usr/local/qemu)" = "$qemu"
test "$(readlink -f /nix/var/nix/gcroots/qemu)" = "$qemu"
outputs=("$qemu")
if [[ "$image" == kernel-dev || "$image" == dev ]]; then
    test "$VDSO_LIBRARY_DIR" = /root/linux_vdso
    test "$(readlink -f "$VDSO_LIBRARY_DIR")" = "$vdso"
    outputs+=("$vdso")
fi
nix-store -qR "${outputs[@]}" > /tmp/runtime-paths
mapfile -t runtime_paths < /tmp/runtime-paths

if [[ "$gc" == yes ]]; then
    # Only OSDK precedes the prebuilt image's automatic-root cleanup.
    if [[ "$image" == osdk-dev ]]; then
        rm -f /nix/var/nix/gcroots/auto/*
    fi
    nix-collect-garbage -d
fi
nix-store --check-validity "${runtime_paths[@]}"

case "$(uname -m)" in
    x86_64) machine="Advanced Micro Devices X86-64" ;;
    aarch64) machine=AArch64 ;;
    *) exit 1 ;;
esac
for arch in x86_64 riscv64 loongarch64 aarch64; do
    program="qemu-system-$arch"
    test "$(readlink -f "$(command -v "$program")")" = "$qemu/bin/$program"
    "$program" --version > /tmp/qemu-version
    test "$(head -n 1 /tmp/qemu-version)" = "QEMU emulator version $qemu_version"
    readelf -h "$qemu/bin/$program" | grep -F "$machine"
    # nixpkgs wraps QEMU to provide its runtime data paths.
    readelf -h "$qemu/bin/.$program-wrapped" | grep -F "$machine"
done
if [[ "$image" == kernel-dev || "$image" == dev ]]; then
    for arch in x86_64 riscv64 aarch64; do
        test -s "$VDSO_LIBRARY_DIR/vdso_$arch.so"
    done
fi
if [[ "$image" != osdk-dev ]]; then
    nix-channel --list > /tmp/channels
    grep -Fx "nixpkgs $nixpkgs_url" /tmp/channels
    grep -Fx "nixos $nixpkgs_url" /tmp/channels
fi
echo "Image checks passed: $image (GC: $gc)"
