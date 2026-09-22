# SPDX-License-Identifier: MPL-2.0

.DEFAULT_GOAL := initramfs-boot-images

TARGET_ARCH ?= x86_64
SMP ?= 1
VERBOSE ?= 1
ENABLE_CONFORMANCE_TEST ?= false
CONFORMANCE_TEST_SUITE ?= ltp
CONFORMANCE_TEST_WORKDIR ?= /tmp
CONFORMANCE_TEST_SELECTOR ?=
ENABLE_REGRESSION_TEST ?= false
# Specify platform macros when building regression tests (supported: asterinas, linux).
# - asterinas: Define both `__asterinas__` and `__linux__`. Tests may fail in Linux.
# - linux: Define only `__linux__`. Tests may fail in Asterinas.
REGRESSION_TEST_PLATFORM ?= asterinas
DNS_SERVER ?= none
MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
BUILD_DIR := $(CUR_DIR)/build
INITRAMFS_DIR := $(BUILD_DIR)/initramfs
ROOTFS_IMAGE := $(BUILD_DIR)/rootfs.img
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
LTP_DEV_IMAGE := $(BUILD_DIR)/ltp_dev.img
SSD_IMAGE := $(BUILD_DIR)/nvme0n1.img
XFSTESTS_TEST_IMAGE := $(BUILD_DIR)/xfstests_test.img
XFSTESTS_SCRATCH_IMAGE := $(BUILD_DIR)/xfstests_scratch.img
XFSTESTS_FS_TYPE ?= ext2
XFSTESTS_DISK_SIZE ?= 12G
include $(CUR_DIR)/src/conformance/xfstests/build_config.mk

# Include benchmark, if BENCHMARK is set.
ifeq ($(BENCHMARK), none)
ENABLE_BENCHMARK_TEST = false
else
ENABLE_BENCHMARK_TEST = true
endif

# Decreases the level of verbosity of diagnostic messages from Nix.
ifeq ($(VERBOSE), 0)
NIX_QUIET = --quiet
endif

NIX := nix --extra-experimental-features 'nix-command flakes'
NIX_BUILD := $(NIX) build --impure --no-update-lock-file \
	--expr '(builtins.getFlake (toString ../..)).lib.mkInitramfs ../..'
# Quote Make values before the shell sees paths or test selectors.
shell_quote = '$(subst ','"'"',$(1))'
GVISOR_LIB_DIR ?= /lib/x86_64-linux-gnu

.PHONY: check-nix-lock
check-nix-lock:
	@$(NIX) flake metadata ../.. --no-update-lock-file >/dev/null

BOOT_IMAGE_NIX_ARGS := \
	--argstr target $(call shell_quote,$(TARGET_ARCH)) \
	--arg enableBenchmarkTest $(ENABLE_BENCHMARK_TEST) \
	--arg enableConformanceTest $(ENABLE_CONFORMANCE_TEST) \
	--arg enableRegressionTest $(ENABLE_REGRESSION_TEST) \
	--argstr benchmarkName $(call shell_quote,$(BENCHMARK)) \
	--argstr conformanceTestSuite $(call shell_quote,$(CONFORMANCE_TEST_SUITE)) \
	--argstr conformanceTestWorkDir $(call shell_quote,$(CONFORMANCE_TEST_WORKDIR)) \
	--argstr conformanceTestSelector $(call shell_quote,$(CONFORMANCE_TEST_SELECTOR)) \
	--argstr regressionTestPlatform $(call shell_quote,$(REGRESSION_TEST_PLATFORM)) \
	--argstr dnsServer $(call shell_quote,$(DNS_SERVER)) \
	--argstr gvisorPrebuiltDir $(call shell_quote,$(GVISOR_PREBUILT_DIR)) \
	--argstr gvisorLibDir $(call shell_quote,$(GVISOR_LIB_DIR)) \
	--arg smp $(SMP)

.PHONY: initramfs-boot-images
ifeq ($(TARGET_ARCH), loongarch64)
initramfs-boot-images: data-disk-images
	@echo "For loongarch, we generate a fake initramfs to successfully test or build."
	@touch $(INITRAMFS_IMAGE)
else
initramfs-boot-images: $(INITRAMFS_IMAGE) data-disk-images
endif

.PHONY: data-disk-images
ifeq ($(TARGET_ARCH), loongarch64)
data-disk-images: $(EXT2_IMAGE) $(EXFAT_IMAGE)
else ifeq ($(XFSTESTS_NEEDS_BLOCK_DEVICES), true)
data-disk-images: $(EXT2_IMAGE) $(EXFAT_IMAGE) $(LTP_DEV_IMAGE) $(SSD_IMAGE) $(XFSTESTS_TEST_IMAGE) $(XFSTESTS_SCRATCH_IMAGE)
else
data-disk-images: $(EXT2_IMAGE) $(EXFAT_IMAGE) $(LTP_DEV_IMAGE) $(SSD_IMAGE)
endif

.PHONY: $(INITRAMFS_IMAGE)
$(INITRAMFS_IMAGE): $(INITRAMFS_DIR)
	@$(NIX_BUILD) \
		$(BOOT_IMAGE_NIX_ARGS) \
		--arg initramfsCompressed $(INITRAMFS_COMPRESSED) \
		--out-link $@ \
		initramfs-image

.PHONY: $(INITRAMFS_DIR)
$(INITRAMFS_DIR): check-nix-lock
	@$(NIX_BUILD) \
		$(BOOT_IMAGE_NIX_ARGS) \
		--out-link $@ \
		initramfs

.PHONY: rootfs-image
rootfs-image: $(ROOTFS_IMAGE)

.PHONY: rootfs-boot-images
ifeq ($(TARGET_ARCH), loongarch64)
rootfs-boot-images:
	$(error Rootfs image is not supported for loongarch64)
else
rootfs-boot-images: rootfs-image data-disk-images
endif

.PHONY: $(ROOTFS_IMAGE)
$(ROOTFS_IMAGE): check-nix-lock
	@$(NIX_BUILD) \
		$(BOOT_IMAGE_NIX_ARGS) \
		--out-link $@ \
		rootfs-image

# Prebuild x86_64 packages
x86_64_pkgs: check-nix-lock
	@$(NIX_BUILD) \
		$(NIX_QUIET) \
		--argstr target x86_64 \
		--arg enableBenchmarkTest true \
		--arg enableConformanceTest true \
		--out-link /nix/var/nix/gcroots/auto/x86_64-pkgs \
		busybox \
		benchmark.fio \
		benchmark.hackbench \
		benchmark.iperf3 \
		benchmark.lmbench \
		benchmark.memcached \
		benchmark.nginx \
		benchmark.redis \
		benchmark.schbench \
		benchmark.sqlite-speedtest1 \
		benchmark.sysbench \
		conformance.ltp \
		conformance.kselftest \
		regression.package

# Prebuild riscv64 packages
# Note: This may cause GitHub Runner ResourceExhausted errors when publish nix docker image.
# If such errors occur, try building each package individually and clear the build cache.
riscv64_pkgs: check-nix-lock
	@$(NIX_BUILD) \
		$(NIX_QUIET) \
		--argstr target riscv64 \
		--arg enableBenchmarkTest false \
		--arg enableConformanceTest true \
		--out-link /nix/var/nix/gcroots/auto/riscv64-pkgs \
		busybox \
		conformance.ltp \
		regression.package

install_host_pkgs: check-nix-lock
	@paths=$$($(NIX_BUILD) --no-link --print-out-paths \
		apacheHttpd iperf3 libmemcached lmbench redis) && nix-env --install $$paths

nix_gc:
	@nix-collect-garbage -d

# This target is used to accelerate CI workflows, e.g., `benchmark_x86` and `benchmark_x86_tdx`.
initramfs_pkgs: check-nix-lock
	@$(NIX_BUILD) \
		$(NIX_QUIET) \
		--argstr target x86_64 \
		--arg enableBenchmarkTest true \
		--out-link /nix/var/nix/gcroots/auto/x86_64-initramfs-with-benchmark \
		initramfs-image

$(EXT2_IMAGE):
	@mkdir -p $(BUILD_DIR)
	@dd if=/dev/zero of=$(EXT2_IMAGE) bs=2G count=1
	@mke2fs $(EXT2_IMAGE)

$(EXFAT_IMAGE):
	@mkdir -p $(BUILD_DIR)
	@fallocate -l 512M $(EXFAT_IMAGE)
	@mkfs.exfat $(EXFAT_IMAGE)

$(LTP_DEV_IMAGE):
	@mkdir -p $(BUILD_DIR)
	@dd if=/dev/zero of=$(LTP_DEV_IMAGE) bs=512M count=1
	@mke2fs -t ext2 -F $(LTP_DEV_IMAGE)

$(SSD_IMAGE):
	@mkdir -p $(BUILD_DIR)
	@dd if=/dev/zero of=$(SSD_IMAGE) bs=256M count=1
	@mke2fs -t ext2 -F $(SSD_IMAGE)

$(XFSTESTS_TEST_IMAGE):
	@mkdir -p $(BUILD_DIR)
	@truncate -s $(XFSTESTS_DISK_SIZE) $@
	@$(XFSTESTS_MKFS) $@

$(XFSTESTS_SCRATCH_IMAGE):
	@mkdir -p $(BUILD_DIR)
	@truncate -s $(XFSTESTS_DISK_SIZE) $@
	@$(XFSTESTS_MKFS) $@

.PHONY: format
format:
	@$(MAKE) --no-print-directory -C src/regression format
	@../../tools/nixfmt.sh nix

.PHONY: check
check:
	@$(MAKE) --no-print-directory -C src/regression check
	@../../tools/nixfmt.sh --check nix

.PHONY: clean
clean:
	@rm -rf $(BUILD_DIR)
