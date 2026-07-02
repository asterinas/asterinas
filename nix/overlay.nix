# SPDX-License-Identifier: MPL-2.0
#
# Asterinas build and toolchain packages layered on nixpkgs.
final: prev:

let inherit (prev) lib stdenv;
in {
  # Rust nightly from rust-toolchain.toml, including components and targets.
  # The shell also carries rust-analyzer from the same nightly; the toml
  # stays the single source of truth and the rustup contract is unchanged.
  asterinas-rust-toolchain = let
    toolchain =
      (builtins.fromTOML (builtins.readFile ../rust-toolchain.toml)).toolchain;
  in final.rust-bin.fromRustupToolchain
  (toolchain // { components = toolchain.components ++ [ "rust-analyzer" ]; });

  # Prebuilt Linux vDSO binaries embedded by the kernel build, pinned to the
  # commit tools/docker/Dockerfile clones.
  asterinas-vdso = final.fetchFromGitHub {
    owner = "asterinas";
    repo = "linux_vdso";
    rev = "74898350d406d6cd8988531ad737380a8e2cdbf4";
    hash = "sha256-Zimwr72fbR694fO7sdYrMK4SDp4w03UHW/QtmJyiP+Q=";
  };

  # OVMF is built through pkgsCross.gnu64, so the edk2 pin must live at the
  # top level and propagate into that package set.
  edk2 = final.callPackage ./packages/edk2.nix { };
}
# Boot-time tools are Linux-only; Darwin gets the build/lint shell.
// lib.optionalAttrs stdenv.isLinux {
  asterinas-qemu = final.callPackage ./packages/qemu.nix { };
  asterinas-grub = final.callPackage ./packages/grub.nix {
    # Match the Docker image's x86_64-efi GRUB build. On non-x86_64 hosts
    # the cross-built tools cannot run locally, so grub.nix pairs these
    # modules with tools from the native build below.
    grub2 = final.pkgsCross.gnu64.grub2.override { efiSupport = true; };
    grub2-host = final.grub2.override { efiSupport = true; };
  };
  asterinas-ovmf = final.callPackage ./packages/ovmf.nix {
    edk2 = final.pkgsCross.gnu64.edk2;
    OVMF = final.pkgsCross.gnu64.OVMF;
  };
  asterinas-klint = final.callPackage ./packages/klint.nix { };
}
