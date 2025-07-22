# SPDX-License-Identifier: MPL-2.0

ARG BASE_VERSION
FROM asterinas/osdk:${BASE_VERSION} AS build-base

SHELL ["/bin/bash", "-c"]

ARG DEBIAN_FRONTEND=noninteractive

# Install Nix package manager
# FIXME: Installing Nix as root is not supported in single-user mode.
RUN sh <(curl -L https://nixos.org/nix/install) --daemon --yes \
    && . /etc/profile.d/nix.sh \
    && nix-channel --add https://nixos.org/channels/nixos-25.05 nixpkgs \
    && nix-channel --update \
    && nix-env -iA nixpkgs.nixfmt \
    && rm /nix/var/nix/gcroots/auto/* \
    && nix-collect-garbage -d

#= Build Nix packages ======================================================

COPY test /root/test
WORKDIR /root/test
ENV PATH="/nix/var/nix/profiles/default/bin:${PATH}"

# Build riscv64 packages
RUN make riscv64_pkgs VERBOSE=0 \
    && make nix_gc

# Build x86_64 packages
RUN make x86_64_pkgs VERBOSE=0 \
    && make nix_gc

# Build general packages and install host required packages
RUN make general_pkgs VERBOSE=0 \
    && make install_host_pkgs \
    && make nix_gc

# Clean source files
RUN rm -rf /root/test

# Let Github runners reuse Nixpkgs tarball installed in the Dockerfile.
RUN mkdir -p /github/home/.cache \
    && ln -s /root/.cache/nix /github/home/.cache/nix

VOLUME [ "/root/asterinas" ]

WORKDIR /root/asterinas
