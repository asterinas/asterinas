{
  description = "Asterinas Operating System";

  inputs = {
    nixpkgs.url = "nixpkgs/nixos-24.05";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, nixpkgs, rust-overlay }:
    let
      supportedSystems = [ "x86_64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
      overlays = [ (import rust-overlay) ];
      nixpkgsFor = forAllSystems (system: import nixpkgs {
        inherit system overlays;
        config.allowUnfree = true;
      });

    in {
      packages = forAllSystems (system:
        let
          pkgs = nixpkgsFor.${system};
        in rec {
          membench = pkgs.callPackage ./test/membench.nix { };
          lmbench = pkgs.callPackage ./test/lmbench.nix { };
          gvisor-syscall-tests-all = pkgs.callPackage ./test/syscall_test/all.nix { };
          gvisor-syscall-tests = pkgs.callPackage ./test/syscall_test/default.nix { inherit gvisor-syscall-tests-all; };
          test-apps = pkgs.callPackage ./test/apps/default.nix { };
          initrd = pkgs.callPackage ./test/initrd.nix { inherit membench lmbench test-apps gvisor-syscall-tests; };

          rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
          rustPlatform = pkgs.makeRustPlatform {
            cargo = rustToolchain;
            rustc = rustToolchain;
          };
          shell = self.devShells.${system}.default;
          cargo-binutils = pkgs.cargo-binutils.override { inherit rustPlatform; };
          typos = pkgs.typos.override { inherit rustPlatform; };

          _cache = pkgs.symlinkJoin {
            name = "_cache";
            paths = with pkgs; [
              # cargo packages
              cargo-binutils typos

              # initrd dependencies
              gvisor-syscall-tests-all
              membench lmbench iozone

              (grub2.override { efiSupport = true; })
            ];
          };
        }
      );

      devShells = forAllSystems (system: let
        pkgs = nixpkgsFor.${system};
        pkgsFlake = self.packages.${system};
      in {
        default = pkgs.mkShell {
          packages = [
            # Rust Toolchain
            pkgsFlake.rustToolchain
            pkgsFlake.cargo-binutils
            pkgsFlake.typos

            # QEMU
            pkgs.qemu

            # Binaries required to build image
            (pkgs.grub2.override { efiSupport = true; })
            pkgs.libisoburn
            pkgs.mtools
            pkgs.exfatprogs
          ];

          shellHook = ''
            export OVMF_PATH=${pkgs.OVMF.fd}/FV
            export PREBUILT_INITRAMFS=${pkgsFlake.initrd}/initrd.gz
          '';
        };
      });
    };
}
