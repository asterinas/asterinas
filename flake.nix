# SPDX-License-Identifier: MPL-2.0
{
  description = "Asterinas development environment";

  inputs = {
    # Keep Nix-based builds on the nixpkgs revision the rest of the repository
    # pins: tools/docker/prebuilt-nix-packages/Dockerfile and
    # test/initramfs/nix/default.nix.
    nixpkgs.url =
      "github:NixOS/nixpkgs/fd1462031fdee08f65fd0b4c6b64e22239a77870";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, rust-overlay }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ];
      forAllSystems = f:
        nixpkgs.lib.genAttrs systems (system:
          f (import nixpkgs {
            inherit system;
            overlays = [ self.overlays.default ];
          }));
    in {
      # rust-overlay is composed in so the overlay is usable on its own.
      overlays.default = nixpkgs.lib.composeExtensions (import rust-overlay)
        (import ./nix/overlay.nix);

      devShells = forAllSystems
        (pkgs: { default = pkgs.callPackage ./nix/devshell.nix { }; });

      packages = forAllSystems (pkgs:
        nixpkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
          qemu = pkgs.asterinas-qemu;
          grub = pkgs.asterinas-grub;
          ovmf = pkgs.asterinas-ovmf;
        });
    };
}
