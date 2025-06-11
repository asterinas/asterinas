# SPDX-License-Identifier: MPL-2.0

ARG BASE_VERSION
FROM asterinas/asterinas:${BASE_VERSION}-nix AS prebuilt-nixpkgs
FROM asterinas/osdk:${BASE_VERSION} AS build-base

SHELL ["/bin/bash", "-c"]

ARG DEBIAN_FRONTEND=noninteractive

#= Install packages for Docker building ====================================

# Please keep the list sorted by name
RUN apt update && apt-get install -y --no-install-recommends \
    apache2-utils \
    clang \
    cmake \
    curl \
    gnupg \
    libevent-dev \
    libslirp-dev \
    libssl-dev \
    jq \
    tcl-dev \
    unzip \
    zip

#= Download dependency =====================================================

WORKDIR /opt/linux_binary_cache
RUN wget https://raw.githubusercontent.com/asterinas/linux_binary_cache/14598b6/vmlinuz-5.15.0-105 \
        -O vmlinuz

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
    libnl-3-dev         `# dependency for netlink socket` \
    libnl-route-3-dev   `# dependency for netlink route socket` \
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

COPY --from=prebuilt-nixpkgs /nix /nix
COPY --from=prebuilt-nixpkgs /etc/profile.d/nix.sh /etc/profile.d/nix.sh
RUN echo "source /etc/profile.d/nix.sh" >> /etc/bash.bashrc \
    && groupadd -r nixbld \
    && for n in $(seq 1 32); do \
           useradd -c "Nix build user $n" -d /var/empty -g nixbld -G nixbld \
           -M -N -r -s "$(which nologin)" nixbld$n; \
       done

# Add the path of Asterinas tools
ENV PATH="/root/asterinas/target/bin:${PATH}"

VOLUME [ "/root/asterinas" ]

WORKDIR /root/asterinas
