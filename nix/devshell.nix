# SPDX-License-Identifier: MPL-2.0
#
# Asterinas dev shell:
#   Linux  : toolchain, cargo tools, boot stack, and host build tools.
#   darwin : build/lint subset. Booting the kernel still needs Linux.
{ lib, mkShell, stdenv, asterinas-rust-toolchain, asterinas-vdso
# Tools osdk/tools/docker/Dockerfile installs with `cargo install`; nixpkgs
# provides them here, so versions may lag the Docker pins.
, cargo-binutils, cargo-expand, lychee, mdbook, mdbook-mermaid, typos
# Host tools used on Linux and Darwin.
, clang, clang-tools, git, python3, yq, jq, gnumake, pkg-config, file
, nixfmt-classic
# Linux-only packages. Only the asterinas-* attrs are absent from the
# package set on Darwin (the overlay defines them for Linux alone).
, asterinas-qemu ? null, asterinas-grub ? null, asterinas-ovmf ? null, gdb
, mtools, xorriso, cpio, dosfstools, exfatprogs, e2fsprogs, util-linux, parted
, socat, strace, virtiofsd, iptables, iproute2, nix, wget, cachix }:

let
  cargoTools =
    [ cargo-binutils cargo-expand lychee mdbook mdbook-mermaid typos ];
  hostCommon = [
    clang
    clang-tools
    git
    python3
    yq
    jq
    gnumake
    pkg-config
    file
    # Match the formatter the prebuilt-nix-packages image installs.
    nixfmt-classic
  ];
  linuxOnly = [
    asterinas-qemu
    asterinas-grub
    asterinas-ovmf
    # Disk-image and filesystem tools the kernel-dev image takes from Ubuntu.
    gdb
    mtools
    xorriso
    cpio
    dosfstools
    exfatprogs
    e2fsprogs
    util-linux
    parted
    socat
    strace
    virtiofsd
    iptables
    iproute2
    # tools/atomic_wget.sh downloads prebuilt artifacts for the benchmarks.
    wget
    # test/initramfs still builds images through nix-build, and
    # `make push_cachix` publishes the distro caches.
    nix
    cachix
  ];
in mkShell {
  packages = [ asterinas-rust-toolchain ] ++ cargoTools ++ hostCommon
    ++ lib.optionals stdenv.isLinux linuxOnly;

  shellHook = ''
    # Use the vDSO checkout pinned by the overlay unless the caller supplied one.
    export VDSO_LIBRARY_DIR="''${VDSO_LIBRARY_DIR:-${asterinas-vdso}}"
  '' + lib.optionalString stdenv.isLinux ''
    # Use the Nix-built firmware unless the caller supplied another OVMF tree.
    export OVMF_DIR="''${OVMF_DIR:-${asterinas-ovmf}}"
  '' + lib.optionalString stdenv.isDarwin ''
    echo "asterinas dev shell (darwin): build/lint only; booting the kernel needs Linux." >&2
  '';
}
