final: prev:
let
  # Use a verified-working Codex version that is newer than NixOS 25.05's
  # package. To update, choose a nixpkgs revision with the desired Codex version
  # and refresh `hash`.
  nixpkgsForCodex = import (prev.fetchFromGitHub {
    owner = "NixOS";
    repo = "nixpkgs";
    rev = "b77b3de8775677f84492abe84635f87b0e153f0f";
    hash = "sha256-nOesoDCiXcUftqbRBMz9tt4blI5PvljMWbm3kuCA+0s=";
  }) {
    inherit (prev) system;
    config = prev.config or { };
  };
in { codex = nixpkgsForCodex.codex; }
