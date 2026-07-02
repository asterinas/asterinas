# SPDX-License-Identifier: MPL-2.0
{
  stdenv,
  mkShell,
  asterinas-rust-toolchain,
  asterinas-vdso,
  # Tools osdk/tools/docker/Dockerfile installs with `cargo install`; nixpkgs
  # provides them here, so versions may lag the Docker pins.
  cargo-binutils,
  cargo-expand,
  cargo-udeps,
  lychee,
  mdbook,
  mdbook-mermaid,
  typos,
  clang,
  clang-tools,
  git,
  python3,
  yq,
  jq,
  gnumake,
  pkg-config,
  file,
  nixfmt,
  asterinas-qemu,
  asterinas-grub,
  asterinas-ovmf,
  gdb,
  mtools,
  xorriso,
  cpio,
  dosfstools,
  exfatprogs,
  e2fsprogs,
  util-linux,
  parted,
  socat,
  strace,
  virtiofsd,
  iptables,
  iproute2,
  nix,
  nixos-install-tools,
  wget,
  cachix,
}:

let
  cargoTools = [
    cargo-binutils
    cargo-expand
    cargo-udeps
    lychee
    mdbook
    mdbook-mermaid
    typos
  ];
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
    nixfmt
  ];
  bootAndHostTools = [
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
    # test/initramfs still builds images through nix-build,
    # `aster-nixos-install` calls `nixos-install` from PATH, and
    # `make push_cachix` publishes the distro caches.
    nix
    nixos-install-tools
    cachix
  ];
  # Host-side clients of the network benchmarks, from the definitions the
  # prebuilt-nix-packages image installs with `make install_host_pkgs`.
  benchmarkHostClients =
    let
      initramfsPkgs = import ../../../test/initramfs/nix {
        target = stdenv.hostPlatform.parsed.cpu.name;
        system = stdenv.hostPlatform.system;
      };
    in
    [
      initramfsPkgs.apacheHttpd
      initramfsPkgs.iperf3
      initramfsPkgs.libmemcached
      initramfsPkgs.lmbench
      initramfsPkgs.redis
    ];
in
mkShell {
  packages = [
    asterinas-rust-toolchain
  ]
  ++ cargoTools
  ++ hostCommon
  ++ bootAndHostTools
  ++ benchmarkHostClients;

  shellHook = ''
    # Change Cargo PATH order so Nix tools precede rustup shims.
    export PATH="$PATH:''${CARGO_HOME:-$HOME/.cargo}/bin"

    # Use the vDSO checkout pinned by the overlay unless the caller supplied one.
    export VDSO_LIBRARY_DIR="''${VDSO_LIBRARY_DIR:-${asterinas-vdso}}"

    # Use the Nix-built firmware unless the caller supplied another OVMF tree.
    export OVMF_DIR="''${OVMF_DIR:-${asterinas-ovmf}}"

    # `make iso` and `make nixos` force linux-efi-handover64, for which the
    # Makefile defaults to the Docker image's /usr/bin/grub-mkrescue.
    export GRUB_MKRESCUE="''${GRUB_MKRESCUE:-${asterinas-grub}/bin/grub-mkrescue}"
  '';
}
