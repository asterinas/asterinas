# SPDX-License-Identifier: MPL-2.0

FROM ubuntu:22.04 AS build-base

ARG DEBIAN_FRONTEND=noninteractive

RUN apt update && apt-get install -y --no-install-recommends \
    build-essential \
    ca-certificates \
    git \
    python3-pip \
    python-is-python3 \
    wget

RUN pip3 install yq tomli

#= Build QEMU =================================================================

FROM build-base AS build-qemu

RUN apt update && apt-get install -y --no-install-recommends \
    libgcrypt-dev   `# optional build dependency` \
    libglib2.0-dev  `# build dependency` \
    libpixman-1-dev `# build dependency` \
    libusb-dev      `# optional build dependency` \
    meson \
    ninja-build
RUN apt clean && rm -rf /var/lib/apt/lists/*

FROM build-qemu AS qemu

# Fetch and install QEMU from the official source
#
# The QEMU version in the Ubuntu 22.04 repository is 6.*, which has a bug to cause OVMF debug to fail.
# The libslirp dependency is for QEMU's network backend.
WORKDIR /root

# FIXME: Update the QEMU version to the latest stable release when available.
RUN wget -O qemu.tar.xz https://download.qemu.org/qemu-10.1.0-rc2.tar.xz \
    && mkdir /root/qemu \
    && tar xf qemu.tar.xz --strip-components=1 -C /root/qemu \
    && rm qemu.tar.xz
WORKDIR /root/qemu
RUN ./configure --target-list=x86_64-softmmu,riscv64-softmmu,loongarch64-softmmu --prefix=/usr/local/qemu --enable-slirp \
    && make -j \
    && make install
WORKDIR /root
RUN rm -rf /root/qemu

#= Build OVMF =================================================================

FROM build-base AS build-ovmf

RUN apt update && apt-get install -y --no-install-recommends \
    bison \
    flex \
    iasl \
    nasm \
    uuid-dev
RUN apt clean && rm -rf /var/lib/apt/lists/*
RUN git --version

FROM build-ovmf AS ovmf

# Fetch and build OVMF from the EDK2 official source
WORKDIR /root
RUN git clone --depth 1 --branch stable/202408 --recurse-submodules --shallow-submodules https://github.com/tianocore/edk2.git
WORKDIR /root/edk2
RUN /bin/bash -c "source ./edksetup.sh \
    && make -C BaseTools \
    && build -a X64 -t GCC5 -b DEBUG -p OvmfPkg/OvmfPkgX64.dsc -D DEBUG_ON_SERIAL_PORT \
    && build -a X64 -t GCC5 -b RELEASE -p OvmfPkg/OvmfPkgX64.dsc"

#= Build GRUB =================================================================

FROM build-base AS build-grub

RUN apt update && apt-get install -y --no-install-recommends \
    autoconf \
    automake \
    autopoint \
    bison \
    flex \
    gawk \
    gettext \
    libfreetype6-dev \
    pkg-config
RUN apt clean && rm -rf /var/lib/apt/lists/*

FROM build-grub AS grub

# Fetch and install GRUB from the GNU official source
#
# We have installed grub-efi-amd64-bin just for the unicode.pf2 file, which is not included
# in the GRUB release. The Ubuntu release notoriously modifies the GRUB source code and enforce
# EFI handover boot, which is deprecated. So we have to build GRUB from source.
WORKDIR /root
# See also: https://github.com/asterinas/asterinas/pull/1710
RUN git clone --single-branch -b asterinas/2.12 https://github.com/asterinas/grub.git \
    && git -C grub checkout 0633bc8
# Fetch and install the Unicode font data for grub.
RUN wget -O unifont.pcf.gz https://ftp.gnu.org/gnu/unifont/unifont-15.1.04/unifont-15.1.04.pcf.gz \
    && mkdir -pv /usr/share/fonts/unifont \
    && gunzip -c unifont.pcf.gz > /usr/share/fonts/unifont/unifont.pcf \
    && rm unifont.pcf.gz
WORKDIR /root/grub

RUN echo depends bli part_gpt > grub-core/extra_deps.lst \
    && ./bootstrap \
    && ./configure \
        --target=x86_64 \
        --disable-efiemu \
        --with-platform=efi \
        --enable-grub-mkfont \
        --prefix=/usr/local/grub \
        --disable-werror \
    && make -j \
    && make install
WORKDIR /root
RUN rm -rf /root/grub

#= The final stages to produce the OSDK development image ====================

FROM build-base AS rust

# Install all OSDK dependent packages
RUN apt update \
    && apt install -y \
    clang \
    curl \
    gdb \
    grub-efi-amd64 \
    grub2-common \
    libpixman-1-dev     `# running dependency for QEMU` \
    mtools              `# used by grub-mkrescue` \
    xorriso \
    && apt clean \
    && rm -rf /var/lib/apt/lists/*

# Install Rust with both nightly and stable
ENV PATH="/root/.cargo/bin:${PATH}"
ARG ASTER_RUST_VERSION
RUN curl https://sh.rustup.rs -sSf | \
        sh -s -- --default-toolchain ${ASTER_RUST_VERSION} -y \
    && rm -rf /root/.cargo/registry && rm -rf /root/.cargo/git \
    && cargo -V \
    && rustup component add rust-src rustc-dev llvm-tools-preview

# Let Github runners reuse the Rust toolchain installed in the Dockerfile and
# also the Cargo crates downloaded subsequently in the Docker build process.
RUN mkdir -p /github/home \
    && ln -s /root/.cargo /github/home/.cargo \
    && ln -s /root/.rustup /github/home/.rustup

# Install cargo tools
RUN cargo install \
    cargo-binutils \
    mdbook \
    typos-cli

# Install QEMU built from the previous stages
COPY --from=qemu /usr/local/qemu /usr/local/qemu
ENV PATH="/usr/local/qemu/bin:${PATH}"
ENV LD_LIBRARY_PATH="/usr/local/qemu/lib/x86_64-linux-gnu:${LD_LIBRARY_PATH}"

# Install OVMF built from the previous stages
COPY --from=ovmf /root/edk2/Build/OvmfX64/DEBUG_GCC5/FV/ /root/ovmf/debug
COPY --from=ovmf /root/edk2/Build/OvmfX64/RELEASE_GCC5/FV/ /root/ovmf/release

# Install GRUB built from the previous stages
COPY --from=grub /usr/local/grub /usr/local/grub
ENV PATH="/usr/local/grub/bin:${PATH}"
# Make a symbolic link for `unicode.pf2` from Ubuntu 22.04 package
RUN ln -sf /usr/share/grub/unicode.pf2 /usr/local/grub/share/grub/unicode.pf2

VOLUME [ "/root/asterinas" ]

WORKDIR /root/asterinas
