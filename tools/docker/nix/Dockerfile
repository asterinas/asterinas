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

COPY test/nix /root/nix
WORKDIR /root
ENV PATH="/nix/var/nix/profiles/default/bin:${PATH}"

# Build riscv64 packages
# Note: This may cause GitHub Runner ResourceExhausted errors. If such errors occur,
# try building each package individually and clear the build cache.
RUN nix-build \
    ./nix/default.nix \
    --quiet -Q \
    --argstr target riscv64 \
    --arg enableBenchmark true \
    --arg enableSyscallTest true \
    --out-link /nix/var/nix/gcroots/auto/riscv64-pkgs \
    -A busybox \
    -A benchmark.fio \
    -A benchmark.hackbench \
    -A benchmark.iperf3 \
    -A benchmark.lmbench \
    -A benchmark.memcached \
    -A benchmark.nginx \
    -A benchmark.redis \
    -A benchmark.sqlite-speedtest1 \
    -A syscall.ltp \
    && nix-collect-garbage -d \
    && nix-store --optimise

# Build x86_64 packages
RUN nix-build \
    ./nix/default.nix \
    --quiet -Q \
    --argstr target x86_64 \
    --arg enableBenchmark true \
    --arg enableSyscallTest true \
    --out-link /nix/var/nix/gcroots/auto/x86_64-pkgs \
    -A busybox \
    -A benchmark.fio \
    -A benchmark.hackbench \
    -A benchmark.iperf3 \
    -A benchmark.lmbench \
    -A benchmark.memcached \
    -A benchmark.nginx \
    -A benchmark.redis \
    -A benchmark.sqlite-speedtest1 \
    -A benchmark.sysbench \
    -A syscall.ltp \
    && nix-collect-garbage -d \
    && nix-store --optimise

# Build general packages and install host required packages
RUN nix-build \
    ./nix/default.nix \
    --quiet -Q \
    --out-link /nix/var/nix/gcroots/auto/linux_vdso \
    -A linux_vdso \
    && nix-env --install \
    --file ./nix/default.nix \
    -A apacheHttpd \
    -A iperf3 \
    -A libmemcached \
    -A lmbench \
    -A redis \
    && nix-collect-garbage -d \
    && nix-store --optimise

# Clean source files
RUN rm -rf /root/nix

VOLUME [ "/root/asterinas" ]

WORKDIR /root/asterinas
