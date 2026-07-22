# SPDX-License-Identifier: MPL-2.0

{ config ? { }, overlays ? [ ], system ? builtins.currentSystem }:
import (builtins.fetchTarball {
  url =
    "https://github.com/NixOS/nixpkgs/archive/c0bebd16e69e631ac6e52d6eb439daba28ac50cd.tar.gz";
  sha256 = "1fbhkqm8cnsxszw4d4g0402vwsi75yazxkpfx3rdvln4n6s68saf";
}) { inherit config overlays system; }
