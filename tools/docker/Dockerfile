# SPDX-License-Identifier: MPL-2.0

ARG BASE_VERSION
FROM asterinas/nix:${BASE_VERSION} AS build-base

SHELL ["/bin/bash", "-c"]

ARG DEBIAN_FRONTEND=noninteractive

#= Download dependency =====================================================

WORKDIR /opt/linux_binary_cache
RUN wget https://raw.githubusercontent.com/asterinas/linux_binary_cache/24db4ff/vmlinuz-6.16.0 \
        -O vmlinuz

#= Download VDSO files =====================================================

WORKDIR /root
RUN git clone https://github.com/asterinas/linux_vdso.git && \
    cd linux_vdso && \
    git checkout 7489835

ENV VDSO_LIBRARY_DIR="/root/linux_vdso"

#= Build syscall test =========================================================

FROM build-base AS build-gvisor

# Install required packages for bazel
RUN apt update && apt install -y curl gnupg2 git \
    python-is-python3 python3 python3-setuptools python3-pip \
    build-essential crossbuild-essential-arm64 qemu-user-static \
    openjdk-11-jdk-headless zip unzip \
    apt-transport-https ca-certificates gnupg-agent \
    software-properties-common \
    pkg-config libffi-dev patch diffutils libssl-dev iptables kmod \
    clang crossbuild-essential-amd64 erofs-utils busybox-static libbpf-dev \
    iproute2 netcat-openbsd libnuma-dev libc6-dev-i386 golang-go

# Install bazel
RUN wget https://github.com/bazelbuild/bazelisk/releases/download/v1.27.0/bazelisk-amd64.deb && \
    dpkg -i bazelisk-amd64.deb && \
    rm bazelisk-amd64.deb

# Build the gvisor syscall test binaries
WORKDIR /root
RUN git clone -b release-20251215.0 --single-branch --depth 1 https://github.com/google/gvisor.git
WORKDIR /root/gvisor
RUN bazel build --test_tag_filters=native //test/syscalls/... && \
    mkdir -p /root/syscall_test_bins && \
    cp bazel-bin/test/syscalls/linux/*_test /root/syscall_test_bins

#= The final stages to produce the Asterinas development image ====================

FROM build-base

# Install all Asterinas dependent packages
RUN apt update && apt-get install -y --no-install-recommends \
    bridge-utils \
    clang-format       `# formatting general tests` \
    cpio \
    cpuid \
    dosfstools \
    exfatprogs \
    file \
    grub-efi-amd64-bin \
    grub-efi-amd64-dbg \
    iptables \
    iproute2 \
    jq \
    net-tools \
    openssh-server \
    parted \
    pkg-config \
    socat \
    strace \
    sudo \
    unzip \
    vim \
    zip
# Clean apt cache
RUN apt clean && rm -rf /var/lib/apt/lists/*

# Install sctrace
RUN ln -s /root/asterinas/tools/sctrace.sh /usr/local/bin/sctrace

# Set the `ASTER_SCML` env var to make sctrace easier to use for Asterinas developers.
# For more information, check out the README of sctrace.
RUN echo 'export ASTER_SCML=$(find /root/asterinas/book/src/kernel/linux-compatibility/ -name "*.scml")' >> /root/.bashrc

# Copy the gvisor syscall test binaries
COPY --from=build-gvisor /root/syscall_test_bins /root/syscall_test_bins
ENV GVISOR_PREBUILT_DIR=/root/syscall_test_bins

# Add the path of Asterinas tools
ENV PATH="/root/asterinas/target/bin:${PATH}"

VOLUME [ "/root/asterinas" ]

WORKDIR /root/asterinas
