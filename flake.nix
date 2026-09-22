# SPDX-License-Identifier: MPL-2.0
{
  description = "Asterinas development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
    # Match typos 1.39.0 from osdk/tools/docker/Dockerfile.
    nixpkgs-typos.url = "github:NixOS/nixpkgs/c5ae371f1a6a7fd27823bc500d9390b38c05fa55";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      nixpkgs-typos,
      rust-overlay,
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems =
        f:
        nixpkgs.lib.genAttrs systems (
          system:
          f (
            import nixpkgs {
              inherit system;
              overlays = [ self.overlays.default ];
            }
          )
        );
    in
    {
      lib.mkInitramfs = sourceRoot: import (sourceRoot + "/test/initramfs/nix") { inherit nixpkgs; };

      # rust-overlay is composed in so the overlay is usable on its own.
      overlays.default = nixpkgs.lib.composeExtensions (import rust-overlay) (
        import ./tools/dev_env/nix/overlay.nix
      );

      devShells = forAllSystems (pkgs: {
        default = pkgs.callPackage ./tools/dev_env/nix/devshell.nix {
          initramfsPkgs = self.lib.mkInitramfs self.outPath {
            target = pkgs.stdenv.hostPlatform.parsed.cpu.name;
            system = pkgs.stdenv.hostPlatform.system;
          };
          typos = nixpkgs-typos.legacyPackages.${pkgs.stdenv.hostPlatform.system}.typos;
        };
      });

      packages = forAllSystems (pkgs: {
        inherit (pkgs) nixfmt nixos-install-tools cachix;
        qemu = pkgs.asterinas-qemu;
        grub = pkgs.asterinas-grub;
        ovmf = pkgs.asterinas-ovmf;
      });
    };
}
