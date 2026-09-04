# SPDX-License-Identifier: MPL-2.0
#
# QEMU pinned to the version and softmmu target list used by the Docker image.
# The binary is host-native; guest architectures run through TCG.
{ qemu, fetchurl }:

(qemu.override {
  hostCpuTargets = [ "x86_64-softmmu" "riscv64-softmmu" "loongarch64-softmmu" ];
  # qemu-ga is a guest-side agent Asterinas never runs, and without nixpkgs'
  # fix-qemu-ga.patch (dropped below) it would exec FHS paths that do not
  # exist on NixOS. Skip it and its `ga` output entirely.
  guestAgentSupport = false;
}).overrideAttrs (old: rec {
  version = "10.2.1";
  src = fetchurl {
    url = "https://download.qemu.org/qemu-${version}.tar.xz";
    hash = "sha256-o3F0d9jiyE1jC//7wg9s0yk+tFqh5trG0MwnaJmRyeE=";
  };

  # nixpkgs' 9.2.x patches only apply to 10.2.1 by fuzz, and they cover guest
  # agent / nested-virtualization cases Asterinas does not use.
  # NOTE: if we need the nested-virtualization in future, we need to apply the nix patch
  patches = [ ];

  # nixpkgs renames VERSION during preConfigure, but QEMU 10.2.1 docs still
  # read ../VERSION. Skip rendered docs for the development environment.
  configureFlags = (old.configureFlags or [ ]) ++ [ "--disable-docs" ];

  # The qemu-kvm alias points at the host softmmu binary, which may not be in
  # this reduced target set. The doc output also needs a directory even though
  # rendered docs are disabled.
  postInstall = (old.postInstall or "") + ''
    rm -f $out/bin/qemu-kvm
    mkdir -p ''${doc:-$out/share/doc}
  '';

  # nixpkgs names qemu-kvm as the main program; it is removed above.
  meta = old.meta // { mainProgram = "qemu-system-x86_64"; };
})
