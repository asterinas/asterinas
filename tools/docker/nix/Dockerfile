# SPDX-License-Identifier: MPL-2.0

FROM ubuntu:22.04 AS build-base

SHELL ["/bin/bash", "-c"]

ARG DEBIAN_FRONTEND=noninteractive

# Install required packages
RUN apt update && apt install -y --no-install-recommends \
    build-essential \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Install Nix package manager
RUN sh <(curl -L https://nixos.org/nix/install) --daemon --yes \
    && . /etc/profile.d/nix.sh \
    && nix-channel --add https://nixos.org/channels/nixos-25.05 nixpkgs \
    && nix-channel --update \
    && nix-env -iA nixpkgs.nixfmt \
    && rm /nix/var/nix/gcroots/auto/* \
    && nix-collect-garbage -d

#= Build Nix packages ======================================================

COPY test/nix /root/nix
WORKDIR /root/nix
ENV PATH="/nix/var/nix/profiles/default/bin:${PATH}"

# Build riscv64 packages sequentially to avoid GitHub Runner ResourceExhausted errors
# TODO: add benchmark packages for riscv64
RUN nix-build \
    --arg pkgs 'import <nixpkgs> { crossSystem = { config = "riscv64-unknown-linux-gnu"; }; }' \
    --quiet -Q \
    --out-link /nix/var/nix/gcroots/auto/riscv64-ltp \
    ltp.nix \
    && nix-collect-garbage -d \
    && nix-store --optimise

RUN nix-build '<nixpkgs>' \
    --arg crossSystem '{ config = "riscv64-unknown-linux-gnu"; }' \
    --quiet -Q \
    --out-link /nix/var/nix/gcroots/auto/riscv64-busybox \
    -A busybox \
    && nix-collect-garbage -d \
    && nix-store --optimise

# Build x86_64 packages
RUN nix-env --install \
    --attr nixpkgs.apacheHttpd \
    --attr nixpkgs.iperf3 \
    --attr nixpkgs.redis \
    && nix-env --install --file libmemcached.nix \
    && nix-env --install --file lmbench.nix \
    && nix-collect-garbage -d \
    && nix-store --optimise

RUN nix-build \
    --arg pkgs 'import <nixpkgs> { crossSystem = { config = "x86_64-unknown-linux-gnu"; }; }' \
    --quiet -Q \
    --out-link /nix/var/nix/gcroots/auto/x86_64-prebuilt-pkgs \
    x86_64-prebuilt-pkgs.nix \
    && nix-collect-garbage -d \
    && nix-store --optimise

WORKDIR /root
