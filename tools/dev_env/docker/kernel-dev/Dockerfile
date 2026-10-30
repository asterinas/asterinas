# SPDX-License-Identifier: MPL-2.0

# This image augments the OSDK development image for Asterinas development.
# Include only tools bound to the Linux-compatible Asterinas kernel, such as
# Linux conformance test suites, Linux binaries, and Linux VDSO files.

ARG BASE_VERSION
FROM asterinas/prebuilt-nix-packages:${BASE_VERSION} AS build-base

SHELL ["/bin/bash", "-c"]

ARG DEBIAN_FRONTEND=noninteractive

#= Download dependency =====================================================

WORKDIR /opt/linux_binary_cache
RUN wget https://raw.githubusercontent.com/asterinas/linux_binary_cache/8e6e8ad382aac7e605d7a058811d09bce478cd7d/6.16.0/vmlinuz \
        -O vmlinuz

#= Download VDSO files =====================================================

WORKDIR /root
RUN git clone https://github.com/asterinas/linux_vdso.git && \
    cd linux_vdso && \
    git checkout 8af5717

ENV VDSO_LIBRARY_DIR="/root/linux_vdso"

#= The final stages to produce the Asterinas development image ====================

FROM build-base

# Install all Asterinas dependent packages
RUN apt update && apt-get install -y --no-install-recommends \
    bridge-utils \
    clang-format        `# formatting general tests` \
    cpio \
    cpuid \
    dosfstools \
    exfatprogs \
    file \
    grub-efi-amd64-bin \
    grub-efi-amd64-dbg \
    iproute2 \
    iptables \
    jq \
    net-tools \
    openssh-server \
    parted \
    pipx \
    socat \
    strace \
    sudo \
    unzip \
    vim \
    virtiofsd \
    zip \
    && apt clean \
    && rm -rf /var/lib/apt/lists/*

# Install yq for Asterinas benchmarks and tests.
ENV PATH="/root/.local/bin:${PATH}"
RUN PIP_NO_CACHE_DIR=1 pipx install yq==3.4.3 \
    && rm -rf /root/.cache/pip /root/.cache/pipx \
    && yq --version

# Install sctrace
RUN ln -s /root/asterinas/tools/sctrace.sh /usr/local/bin/sctrace

# Set the `ASTER_SCML` env var to make sctrace easier to use for Asterinas developers.
# For more information, check out the README of sctrace.
RUN echo 'export ASTER_SCML=$(find /root/asterinas/book/src/kernel/linux-compatibility/ -name "*.scml" 2>/dev/null)' >> /root/.bashrc

# Add the path of Asterinas tools
ENV PATH="/root/asterinas/target/bin:${PATH}"

VOLUME [ "/root/asterinas" ]

WORKDIR /root/asterinas
