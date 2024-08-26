{
  description = "Asterinas Operating System";

  inputs = {
    nixpkgs.url = "nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      supportedSystems = [ "x86_64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
      overlays = [
        self.overlay
      ];
      nixpkgsFor = forAllSystems (system: import nixpkgs {
        inherit system overlays;
        config.allowUnfree = true;
      });

    in {
      overlay = final: prev: {
      };

      packages = forAllSystems (system:
        let
          pkgs = nixpkgsFor.${system};
        in rec {
          initrd = (pkgs.callPackage ./test/initrd.nix { });
        }
      );
    };
}
