# SPDX-License-Identifier: MPL-2.0

{
  config ? { },
  overlays ? [ ],
  system ? builtins.currentSystem,
  crossSystem ? null,
}:
let
  lock = builtins.fromJSON (builtins.readFile ../flake.lock);
  node = lock.nodes.${lock.root}.inputs.nixpkgs;
  source = lock.nodes.${node}.locked;
  nixpkgs = builtins.fetchTarball {
    url = "https://github.com/${source.owner}/${source.repo}/archive/${source.rev}.tar.gz";
    sha256 = source.narHash;
  };
in
import nixpkgs {
  inherit
    config
    overlays
    system
    crossSystem
    ;
}
