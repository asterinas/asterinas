final: prev:
let
  # Use the Codex package from the pinned NixOS 26.05 package set.
  # To update, refresh the shared nixpkgs pin in `distro/nixpkgs.nix`.
  nixpkgsForCodex = import ../../../nixpkgs.nix {
    inherit (prev.stdenv.hostPlatform) system;
    config = prev.config or { };
  };
in { codex = nixpkgsForCodex.codex; }
