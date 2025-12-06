{ ... }:
let
  # Pinned nixpkgs (channel: nixos-25.05, release date: 2025-07-01)
  pkgs = import (fetchTarball
    "https://github.com/NixOS/nixpkgs/archive/c0bebd16e69e631ac6e52d6eb439daba28ac50cd.tar.gz") {
      config = { allowUnfree = true; };
      overlays = [
        (import ./overlays/hello-asterinas/default.nix)
        (import ./overlays/desktop/default.nix)
        (import ./overlays/podman/default.nix)
      ];
    };
  pushToCachix = with pkgs; [
    hello-asterinas
    xfce.xfdesktop
    xfce.xfwm4
    xorg.xorgserver
    runc
    podman
  ];
in pkgs.writeClosure pushToCachix
