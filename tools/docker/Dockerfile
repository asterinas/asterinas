# SPDX-License-Identifier: MPL-2.0

ARG BASE_VERSION
FROM asterinas/nix:${BASE_VERSION} AS build-base

SHELL ["/bin/bash", "-c"]

ARG DEBIAN_FRONTEND=noninteractive

#= Download dependency =====================================================

WORKDIR /opt/linux_binary_cache
RUN wget https://raw.githubusercontent.com/asterinas/linux_binary_cache/14598b6/vmlinuz-5.15.0-105 \
        -O vmlinuz

#= Build syscall test =========================================================

FROM build-base AS build-bazel

# Install bazel, which is required by the system call test suite from Gvisor project
RUN mkdir -p /root/bazel
COPY tools/install_bazel.sh /root/bazel/
WORKDIR /root/bazel
RUN ./install_bazel.sh
RUN apt clean && rm -rf /var/lib/apt/lists/*
WORKDIR /root
RUN rm -rf bazel

FROM build-bazel AS build-gvisor

# Build the gvisor syscall test binaries
COPY test/src/syscall/gvisor /root/gvisor
WORKDIR /root/gvisor
RUN export BUILD_DIR=build && \
    make ${BUILD_DIR}/syscall_test_bins

#= The final stages to produce the Asterinas development image ====================

FROM build-base

# Install all Asterinas dependent packages
RUN apt update && apt-get install -y --no-install-recommends \
    bridge-utils \
    clang-format       `# formatting general tests` \
    cpio \
    cpuid \
    exfatprogs \
    file \
    grub-efi-amd64-bin \
    grub-efi-amd64-dbg \
    iptables \
    iproute2 \
    jq \
    net-tools \
    openssh-server \
    pkg-config \
    socat \
    strace \
    sudo \
    unzip \
    vim \
    zip
# Clean apt cache
RUN apt clean && rm -rf /var/lib/apt/lists/*

# Copy the gvisor syscall test binaries
COPY --from=build-gvisor /root/gvisor/build/syscall_test_bins /root/syscall_test_bins
ENV GVISOR_PREBUILT_DIR=/root/syscall_test_bins

# Add the path of Asterinas tools
ENV PATH="/root/asterinas/target/bin:${PATH}"

VOLUME [ "/root/asterinas" ]

WORKDIR /root/asterinas
