# SPDX-License-Identifier: MPL-2.0

{ config ? { }, overlays ? [ ], system ? builtins.currentSystem }:
import (builtins.fetchTarball {
  url =
    "https://github.com/NixOS/nixpkgs/archive/fd1462031fdee08f65fd0b4c6b64e22239a77870.tar.gz";
  sha256 = "0h0snjjawavy0gl176iyxqdcmv85vx3nlm0aalwr1q8m2960ly4z";
}) { inherit config overlays system; }
