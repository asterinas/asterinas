# SPDX-License-Identifier: MPL-2.0

{ config ? { }, overlays ? [ ], system ? builtins.currentSystem }:
import (builtins.fetchTarball {
  url =
    "https://github.com/NixOS/nixpkgs/archive/4062d36ebeae843c750011eef6b61ec9a9dbc9a9.tar.gz";
  sha256 = "0hha7lam2c2655f7m0w9jkn8pacmprzgcg3fg7jrnv479fcdh8y2";
}) { inherit config overlays system; }
