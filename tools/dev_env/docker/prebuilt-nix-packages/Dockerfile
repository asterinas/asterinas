# SPDX-License-Identifier: MPL-2.0

# This image adds Nix tooling and cached Asterinas test packages to the OSDK image.

ARG BASE_VERSION
FROM asterinas/osdk-dev:${BASE_VERSION} AS build-base

SHELL ["/bin/bash", "-c"]

ARG DEBIAN_FRONTEND=noninteractive

# Register channels so `nix-shell -p` works in the image.
COPY flake.lock /tmp/asterinas/flake.lock
RUN <<'EOF'
set -e
nixpkgs_url="$(python3 - <<'PYTHON'
import json

with open("/tmp/asterinas/flake.lock") as f:
    lock = json.load(f)
node = lock["nodes"][lock["root"]]["inputs"]["nixpkgs"]
source = lock["nodes"][node]["locked"]
print(f"https://github.com/{source['owner']}/{source['repo']}/archive/{source['rev']}.tar.gz")
PYTHON
)"
nix-channel --add "$nixpkgs_url" nixpkgs
nix-channel --add "$nixpkgs_url" nixos
nix-channel --update
nix-env -iA nixpkgs.nixfmt
nix-env -iA nixpkgs.nixos-install-tools
nix-env -iA cachix -f https://cachix.org/api/v1/install
rm /nix/var/nix/gcroots/auto/*
nix-collect-garbage -d
EOF

#= Build Nix packages ======================================================

COPY test/initramfs /tmp/asterinas/test/initramfs
COPY distro/nixpkgs.nix /tmp/asterinas/distro/nixpkgs.nix
WORKDIR /tmp/asterinas/test/initramfs

# Build aarch64 packages
RUN make aarch64_pkgs VERBOSE=0 \
    && make nix_gc

# Build riscv64 packages
RUN make riscv64_pkgs VERBOSE=0 \
    && make nix_gc

# Build x86_64 packages
RUN make x86_64_pkgs VERBOSE=0 \
    && make nix_gc

# Install host required packages
RUN make install_host_pkgs \
    && make nix_gc

# Build initramfs packages to accelerate CI workflows
#
# It could also cache `stdenvNoCC`, a fundamental and predefined build environment.
# Therefore, `make nix_gc` MUST NOT be run after this.
RUN make initramfs_pkgs VERBOSE=0

# Clean source files
RUN rm -rf /tmp/asterinas

# Let Github runners reuse Nixpkgs tarball installed in the Dockerfile.
RUN mkdir -p /github/home/.cache \
    && ln -s /root/.cache/nix /github/home/.cache/nix

VOLUME [ "/root/asterinas" ]

WORKDIR /root/asterinas
