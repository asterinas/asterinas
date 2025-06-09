# SPDX-License-Identifier: MPL-2.0

ARG BASE_VERSION
FROM asterinas/osdk:${BASE_VERSION} AS build-base

# Fetch and install QEMU from the intel-staging/qemu-tdx source
FROM build-base AS build-qemu-tdx

RUN apt update && apt-get install -y --no-install-recommends \
    libgcrypt-dev   `# optional build dependency` \
    libglib2.0-dev  `# build dependency` \
    libpixman-1-dev `# build dependency` \
    libusb-dev      `# optional build dependency` \
    meson \
    ninja-build
RUN apt clean && rm -rf /var/lib/apt/lists/*

FROM build-qemu-tdx AS qemu-tdx

WORKDIR /root
RUN git clone --depth 1 -b tdx-qemu-upstream-2024.02.29-v8.2.0 https://github.com/intel-staging/qemu-tdx.git
WORKDIR /root/qemu-tdx
COPY osdk/tools/docker/tdx/tdx_qemu.patch /root/qemu-tdx
RUN git apply tdx_qemu.patch \
    && mkdir build \
    && cd build \
    && ../configure --enable-kvm --target-list=x86_64-softmmu --prefix=/usr/local/qemu --enable-slirp \
    && make -j \
    && make install
WORKDIR /root
RUN rm -rf /root/qemu-tdx

FROM build-base

# Install QEMU built from the previous stages
COPY --from=qemu-tdx /usr/local/qemu /usr/local/qemu

WORKDIR /root/asterinas
