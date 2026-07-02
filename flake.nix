# SPDX-License-Identifier: MPL-2.0
{
  description = "Asterinas development environment";

  inputs = {
    # Keep Nix-based builds on the nixpkgs revision used by the Docker image.
    nixpkgs.url =
      "github:NixOS/nixpkgs/c0bebd16e69e631ac6e52d6eb439daba28ac50cd";
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
          klint = pkgs.asterinas-klint;
        });
    };
}
