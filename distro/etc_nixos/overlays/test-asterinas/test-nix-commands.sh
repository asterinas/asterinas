#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

# Test nix-env
nix-env -iA nixos.hello
hello
nix-env -e hello

# Test nix-shell
nix-shell -p hello --command hello

# Test nix-build
nix-build "<nixpkgs>" -A hello
./result/bin/hello

# Test nixos-rebuild
sed -i "s/environment.systemPackages = with pkgs; \[ test-asterinas \];/environment.systemPackages = with pkgs; \[ test-asterinas hello \];/" \
  /etc/nixos/configuration.nix
nixos-rebuild test
# Clean the hash cache to use the hello installed by nixos-rebuild
hash -r
hello
