# SPDX-License-Identifier: MPL-2.0

# This image adds Nix tooling and cached Asterinas test packages to the OSDK image.

ARG BASE_VERSION
FROM asterinas/osdk-dev:${BASE_VERSION} AS build-base

SHELL ["/bin/bash", "-c"]

ARG DEBIAN_FRONTEND=noninteractive

# FIXME: Installing Nix as root is not supported in single-user mode.
RUN sh <(curl -L https://nixos.org/nix/install) --daemon --yes --no-channel-add

COPY flake.nix flake.lock rust-toolchain.toml /tmp/asterinas/
COPY tools/dev_env/nix /tmp/asterinas/tools/dev_env/nix
COPY test/initramfs /tmp/asterinas/test/initramfs
WORKDIR /tmp/asterinas
ENV PATH="/nix/var/nix/profiles/default/bin:${PATH}"

# Install explicit store outputs into the existing profile so GC retains them.
RUN . /etc/profile.d/nix.sh \
    && nix --extra-experimental-features 'nix-command flakes' flake metadata --no-update-lock-file \
    && paths=$(nix --extra-experimental-features 'nix-command flakes' build \
        --no-update-lock-file --no-link --print-out-paths \
        .#nixfmt .#nixos-install-tools .#cachix) \
    && nix-env --install $paths \
    && nix-collect-garbage -d

WORKDIR /tmp/asterinas/test/initramfs

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

# Let GitHub runners reuse the input fetcher cache.
RUN mkdir -p /github/home/.cache \
    && ln -s /root/.cache/nix /github/home/.cache/nix

VOLUME [ "/root/asterinas" ]

WORKDIR /root/asterinas
