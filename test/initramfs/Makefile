# SPDX-License-Identifier: MPL-2.0

OSDK_TARGET_ARCH ?= x86_64
SMP ?= 1
VERBOSE ?= 1
SYSCALL_TEST_SUITE ?= ltp
SYSCALL_TEST_WORKDIR ?= /tmp
ENABLE_BASIC_TEST ?= false
# Specify platform macros when building regression tests (supported: asterinas, linux).
# - asterinas: Define both `__asterinas__` and `__linux__`. Tests may fail in Linux.
# - linux: Define only `__linux__`. Tests may fail in Asterinas.
BASIC_TEST_PLATFORM ?= asterinas
DNS_SERVER ?= none
# Set Nix's cached tarballs to be live for a longer period of time (30 days) to avoid network traffics.
# Nix's default value is rather small (1 hour or 3600 seconds).
NIXPKGS_CACHE_TTL := 2592000 # In seconds
MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
BUILD_DIR := $(CUR_DIR)/build
INITRAMFS := $(BUILD_DIR)/initramfs
INITRAMFS_SKIP_GZIP ?= 0
ifeq ($(INITRAMFS_SKIP_GZIP),1)
INITRAMFS_IMAGE := $(BUILD_DIR)/initramfs.cpio
INITRAMFS_COMPRESSED := false
else
INITRAMFS_IMAGE := $(BUILD_DIR)/initramfs.cpio.gz
INITRAMFS_COMPRESSED := true
endif
EXT2_IMAGE := $(BUILD_DIR)/ext2.img
EXFAT_IMAGE := $(BUILD_DIR)/exfat.img

# Include benchmark, if BENCHMARK is set.
ifeq ($(BENCHMARK), none)
ENABLE_BENCHMARK = false
else
ENABLE_BENCHMARK = true
endif

# Include syscall_test, if BUILD_SYSCALL_TEST is set.
ifeq ($(BUILD_SYSCALL_TEST), 1)
ENABLE_SYSCALL_TEST = true
else
ENABLE_SYSCALL_TEST = false
endif

# Decreases the level of verbosity of diagnostic messages from Nix.
ifeq ($(VERBOSE), 0)
NIX_QUIET = --quiet -Q
endif

.PHONY: all
all: build

.PHONY: build
ifeq ($(OSDK_TARGET_ARCH), loongarch64)
build: $(EXT2_IMAGE) $(EXFAT_IMAGE)
	@echo "For loongarch, we generate a fake initramfs to successfully test or build."
	@touch $(INITRAMFS_IMAGE)
else
build: $(INITRAMFS_IMAGE) $(EXT2_IMAGE) $(EXFAT_IMAGE)
endif

.PHONY: $(INITRAMFS_IMAGE)
$(INITRAMFS_IMAGE): $(INITRAMFS)
	@nix-build \
		--tarball-ttl $(NIXPKGS_CACHE_TTL) \
		--argstr target $(OSDK_TARGET_ARCH) \
		--arg enableBasicTest $(ENABLE_BASIC_TEST) \
		--argstr basicTestPlatform $(BASIC_TEST_PLATFORM) \
		--arg enableBenchmark $(ENABLE_BENCHMARK) \
		--arg enableSyscallTest $(ENABLE_SYSCALL_TEST) \
		--argstr syscallTestSuite $(SYSCALL_TEST_SUITE) \
		--argstr syscallTestWorkDir $(SYSCALL_TEST_WORKDIR) \
		--argstr dnsServer ${DNS_SERVER} \
		--arg initramfsCompressed $(INITRAMFS_COMPRESSED) \
		--arg smp $(SMP) \
		--out-link $@ \
		nix -A initramfs-image

.PHONY: $(INITRAMFS)
$(INITRAMFS):
	@nix-build \
		--tarball-ttl $(NIXPKGS_CACHE_TTL) \
		--argstr target $(OSDK_TARGET_ARCH) \
		--arg enableBasicTest $(ENABLE_BASIC_TEST) \
		--argstr basicTestPlatform $(BASIC_TEST_PLATFORM) \
		--arg enableBenchmark $(ENABLE_BENCHMARK) \
		--arg enableSyscallTest $(ENABLE_SYSCALL_TEST) \
		--argstr syscallTestSuite $(SYSCALL_TEST_SUITE) \
		--argstr syscallTestWorkDir $(SYSCALL_TEST_WORKDIR) \
		--argstr dnsServer ${DNS_SERVER} \
		--arg smp $(SMP) \
		--out-link $@ \
		nix -A initramfs

# Prebuild x86_64 packages
x86_64_pkgs:
	@nix-build \
		nix/default.nix \
		--tarball-ttl $(NIXPKGS_CACHE_TTL) \
		$(NIX_QUIET) \
		--argstr target x86_64 \
		--arg enableBenchmark true \
		--arg enableSyscallTest true \
		--out-link /nix/var/nix/gcroots/auto/x86_64-pkgs \
		-A busybox \
		-A apps.package \
		-A benchmark.fio \
		-A benchmark.hackbench \
		-A benchmark.iperf3 \
		-A benchmark.lmbench \
		-A benchmark.memcached \
		-A benchmark.nginx \
		-A benchmark.redis \
		-A benchmark.schbench \
		-A benchmark.sqlite-speedtest1 \
		-A benchmark.sysbench \
		-A syscall.ltp

# Prebuild riscv64 packages
# Note: This may cause GitHub Runner ResourceExhausted errors when publish nix docker image.
# If such errors occur, try building each package individually and clear the build cache.
riscv64_pkgs:
	@nix-build \
		nix/default.nix \
		--tarball-ttl $(NIXPKGS_CACHE_TTL) \
		$(NIX_QUIET) \
		--argstr target riscv64 \
		--arg enableBenchmark false \
		--arg enableSyscallTest true \
		--out-link /nix/var/nix/gcroots/auto/riscv64-pkgs \
		-A busybox \
		-A apps.package \
		-A syscall.ltp

general_pkgs:
	@nix-build \
		nix/default.nix \
		--tarball-ttl $(NIXPKGS_CACHE_TTL) \
		$(NIX_QUIET) \
		--out-link /nix/var/nix/gcroots/auto/general \
		-A apps.mongoose_src

install_host_pkgs:
	@nix-env \
		--file nix/default.nix \
		--tarball-ttl $(NIXPKGS_CACHE_TTL) \
		--install \
		-A apacheHttpd \
		-A iperf3 \
		-A libmemcached \
		-A lmbench \
		-A redis

nix_gc:
	@nix-collect-garbage -d

# This target is used to accelerate CI workflows, e.g., `benchmark_x86` and `benchmark_x86_tdx`.
initramfs_pkgs:
	@nix-build \
		nix/default.nix \
		--tarball-ttl $(NIXPKGS_CACHE_TTL) \
		$(NIX_QUIET) \
		--argstr target x86_64 \
		--arg enableBenchmark true \
		--out-link /nix/var/nix/gcroots/auto/x86_64-initramfs-with-benchmark \
		-A initramfs-image

$(EXT2_IMAGE):
	@mkdir -p $(BUILD_DIR)
	@dd if=/dev/zero of=$(EXT2_IMAGE) bs=2G count=1
	@mke2fs $(EXT2_IMAGE)

$(EXFAT_IMAGE):
	@mkdir -p $(BUILD_DIR)
	@fallocate -l 64M $(EXFAT_IMAGE)
	@mkfs.exfat $(EXFAT_IMAGE)

.PHONY: format
format:
	@$(MAKE) --no-print-directory -C src/apps format
	@nixfmt nix

.PHONY: check
check:
	@$(MAKE) --no-print-directory -C src/apps check
	@nixfmt --check nix

.PHONY: clean
clean:
	@rm -rf $(BUILD_DIR)
