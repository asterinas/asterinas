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
RUN nix-build '<nixpkgs>' \
    --arg crossSystem '{ config = "riscv64-unknown-linux-gnu"; }' \
    --quiet -Q \
    --out-link /nix/var/nix/gcroots/auto/riscv64-unixbench \
    -A unixbench \
    && nix-collect-garbage -d \
    && nix-store --optimise

RUN nix-build '<nixpkgs>' \
    --arg crossSystem '{ config = "riscv64-unknown-linux-gnu"; }' \
    --quiet -Q \
    --out-link /nix/var/nix/gcroots/auto/riscv64-fio \
    -A fio \
    && nix-collect-garbage -d \
    && nix-store --optimise

RUN nix-build '<nixpkgs>' \
    --arg crossSystem '{ config = "riscv64-unknown-linux-gnu"; }' \
    --quiet -Q \
    --out-link /nix/var/nix/gcroots/auto/riscv64-libmemcached \
    -A libmemcached \
    && nix-collect-garbage -d \
    && nix-store --optimise

RUN nix-build \
    --arg pkgs 'import <nixpkgs> { crossSystem = { config = "riscv64-unknown-linux-gnu"; }; }' \
    --quiet -Q \
    --out-link /nix/var/nix/gcroots/auto/riscv64-ltp \
    ltp.nix \
    && nix-collect-garbage -d \
    && nix-store --optimise

RUN nix-build \
    --arg pkgs 'import <nixpkgs> { crossSystem = { config = "riscv64-unknown-linux-gnu"; }; }' \
    --quiet -Q \
    --out-link /nix/var/nix/gcroots/auto/riscv64-lmbench \
    lmbench.nix \
    && nix-collect-garbage -d \
    && nix-store --optimise

RUN nix-build '<nixpkgs>' \
    --arg crossSystem '{ config = "riscv64-unknown-linux-gnu"; }' \
    --quiet -Q \
    --out-link /nix/var/nix/gcroots/auto/riscv64-iperf3 \
    -A iperf3 \
    && nix-collect-garbage -d \
    && nix-store --optimise

RUN nix-build \
    --arg pkgs 'import <nixpkgs> { crossSystem = { config = "riscv64-unknown-linux-gnu"; }; }' \
    --quiet -Q \
    --out-link /nix/var/nix/gcroots/auto/riscv64-hackbench \
    hackbench.nix \
    && nix-collect-garbage -d \
    && nix-store --optimise

RUN nix-build \
    --arg pkgs 'import <nixpkgs> { crossSystem = { config = "riscv64-unknown-linux-gnu"; }; }' \
    --quiet -Q \
    --out-link /nix/var/nix/gcroots/auto/riscv64-iozone \
    iozone.nix \
    && nix-collect-garbage -d \
    && nix-store --optimise

RUN nix-build \
    --arg pkgs 'import <nixpkgs> { crossSystem = { config = "riscv64-unknown-linux-gnu"; }; }' \
    --quiet -Q \
    --out-link /nix/var/nix/gcroots/auto/riscv64-membench\
    membench.nix \
    && nix-collect-garbage -d \
    && nix-store --optimise

RUN nix-build \
    --arg pkgs 'import <nixpkgs> { crossSystem = { config = "riscv64-unknown-linux-gnu"; }; }' \
    --quiet -Q \
    --out-link /nix/var/nix/gcroots/auto/riscv64-schbench \
    schbench.nix \
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
RUN nix-build \
    --arg pkgs 'import <nixpkgs> { crossSystem = { config = "x86_64-unknown-linux-gnu"; }; }' \
    --quiet -Q \
    --out-link /nix/var/nix/gcroots/auto/x86_64-prebuilt-pkgs \
    prebuilt-pkgs.nix \
    && nix-collect-garbage -d \
    && nix-store --optimise

WORKDIR /root
