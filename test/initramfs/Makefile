# SPDX-License-Identifier: MPL-2.0

TARGET_ARCH ?= x86_64
SMP ?= 1
VERBOSE ?= 1
ENABLE_CONFORMANCE_TEST ?= false
CONFORMANCE_TEST_SUITE ?= ltp
CONFORMANCE_TEST_WORKDIR ?= /tmp
ENABLE_REGRESSION_TEST ?= false
# Specify platform macros when building regression tests (supported: asterinas, linux).
# - asterinas: Define both `__asterinas__` and `__linux__`. Tests may fail in Linux.
# - linux: Define only `__linux__`. Tests may fail in Asterinas.
REGRESSION_TEST_PLATFORM ?= asterinas
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
EXT4_IMAGE := $(BUILD_DIR)/ext4.img
EXT4_SEED_FILE := $(CUR_DIR)/ext4-root/seed.txt
# Feature-matrix images for the ext regression suite: one image per
# `mke2fs -O` shape the unified ext4 driver must accept (or reject).
EXT_MATRIX_DIR := $(BUILD_DIR)/ext_matrix
EXT_MATRIX_IMAGES := \
	$(EXT_MATRIX_DIR)/ext2.img \
	$(EXT_MATRIX_DIR)/ext2_i128.img \
	$(EXT_MATRIX_DIR)/ext2_i128_nori.img \
	$(EXT_MATRIX_DIR)/ext4_journal.img \
	$(EXT_MATRIX_DIR)/ext4_noextents.img \
	$(EXT_MATRIX_DIR)/neg_nofiletype.img \
	$(EXT_MATRIX_DIR)/neg_rev0.img \
	$(EXT_MATRIX_DIR)/neg_metadata_csum.img
EXFAT_IMAGE := $(BUILD_DIR)/exfat.img
LTP_DEV_IMAGE := $(BUILD_DIR)/ltp_dev.img
SSD_IMAGE := $(BUILD_DIR)/nvme0n1.img
XFSTESTS_TEST_IMAGE := $(BUILD_DIR)/xfstests_test.img
XFSTESTS_SCRATCH_IMAGE := $(BUILD_DIR)/xfstests_scratch.img
XFSTESTS_DISK_SIZE ?= 12G

# Include benchmark, if BENCHMARK is set.
ifeq ($(BENCHMARK), none)
ENABLE_BENCHMARK_TEST = false
else
ENABLE_BENCHMARK_TEST = true
endif

BUILD_XFSTESTS_IMAGES = false
ifeq ($(ENABLE_CONFORMANCE_TEST), true)
ifeq ($(CONFORMANCE_TEST_SUITE), xfstests)
BUILD_XFSTESTS_IMAGES = true
endif
endif

REGRESSION_IMAGES =
ifeq ($(ENABLE_REGRESSION_TEST), true)
REGRESSION_IMAGES += $(EXT4_IMAGE)
REGRESSION_IMAGES += $(EXT_MATRIX_IMAGES)
endif

# Decreases the level of verbosity of diagnostic messages from Nix.
ifeq ($(VERBOSE), 0)
NIX_QUIET = --quiet -Q
endif

.PHONY: all
all: build

.PHONY: build
ifeq ($(TARGET_ARCH), loongarch64)
build: $(EXT2_IMAGE) $(EXFAT_IMAGE) $(REGRESSION_IMAGES)
	@echo "For loongarch, we generate a fake initramfs to successfully test or build."
	@touch $(INITRAMFS_IMAGE)
else ifeq ($(BUILD_XFSTESTS_IMAGES), true)
build: $(INITRAMFS_IMAGE) $(EXT2_IMAGE) $(EXFAT_IMAGE) $(LTP_DEV_IMAGE) $(SSD_IMAGE) $(XFSTESTS_TEST_IMAGE) $(XFSTESTS_SCRATCH_IMAGE) $(REGRESSION_IMAGES)
else
build: $(INITRAMFS_IMAGE) $(EXT2_IMAGE) $(EXFAT_IMAGE) $(LTP_DEV_IMAGE) $(SSD_IMAGE) $(REGRESSION_IMAGES)
endif

.PHONY: $(INITRAMFS_IMAGE)
$(INITRAMFS_IMAGE): $(INITRAMFS)
	@nix-build \
		--tarball-ttl $(NIXPKGS_CACHE_TTL) \
		--argstr target $(TARGET_ARCH) \
		--arg enableBenchmarkTest $(ENABLE_BENCHMARK_TEST) \
		--arg enableConformanceTest $(ENABLE_CONFORMANCE_TEST) \
		--arg enableRegressionTest $(ENABLE_REGRESSION_TEST) \
		--argstr conformanceTestSuite $(CONFORMANCE_TEST_SUITE) \
		--argstr conformanceTestWorkDir $(CONFORMANCE_TEST_WORKDIR) \
		--argstr regressionTestPlatform $(REGRESSION_TEST_PLATFORM) \
		--argstr dnsServer ${DNS_SERVER} \
		--arg initramfsCompressed $(INITRAMFS_COMPRESSED) \
		--arg smp $(SMP) \
		--out-link $@ \
		nix -A initramfs-image

.PHONY: $(INITRAMFS)
$(INITRAMFS):
	@nix-build \
		--tarball-ttl $(NIXPKGS_CACHE_TTL) \
		--argstr target $(TARGET_ARCH) \
		--arg enableBenchmarkTest $(ENABLE_BENCHMARK_TEST) \
		--arg enableConformanceTest $(ENABLE_CONFORMANCE_TEST) \
		--arg enableRegressionTest $(ENABLE_REGRESSION_TEST) \
		--argstr conformanceTestSuite $(CONFORMANCE_TEST_SUITE) \
		--argstr conformanceTestWorkDir $(CONFORMANCE_TEST_WORKDIR) \
		--argstr regressionTestPlatform $(REGRESSION_TEST_PLATFORM) \
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
		--arg enableBenchmarkTest true \
		--arg enableConformanceTest true \
		--out-link /nix/var/nix/gcroots/auto/x86_64-pkgs \
		-A busybox \
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
		-A conformance.ltp \
		-A conformance.kselftest \
		-A regression.package

# Prebuild riscv64 packages
# Note: This may cause GitHub Runner ResourceExhausted errors when publish nix docker image.
# If such errors occur, try building each package individually and clear the build cache.
riscv64_pkgs:
	@nix-build \
		nix/default.nix \
		--tarball-ttl $(NIXPKGS_CACHE_TTL) \
		$(NIX_QUIET) \
		--argstr target riscv64 \
		--arg enableBenchmarkTest false \
		--arg enableConformanceTest true \
		--out-link /nix/var/nix/gcroots/auto/riscv64-pkgs \
		-A busybox \
		-A conformance.ltp \
		-A regression.package

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
		--arg enableBenchmarkTest true \
		--out-link /nix/var/nix/gcroots/auto/x86_64-initramfs-with-benchmark \
		-A initramfs-image

$(EXT2_IMAGE):
	@mkdir -p $(BUILD_DIR)
	@dd if=/dev/zero of=$(EXT2_IMAGE) bs=2G count=1
	@mke2fs $(EXT2_IMAGE)

.PHONY: $(EXT4_IMAGE)
$(EXT4_IMAGE): $(EXT4_SEED_FILE)
	@mkdir -p $(BUILD_DIR)
	@truncate -s 128M $(EXT4_IMAGE)
	@mke2fs -q -F -t ext4 -b 4096 -I 256 \
		-O extent,filetype,^metadata_csum,^dir_index,^64bit,^flex_bg,^inline_data,^has_journal,^resize_inode \
		-d $(CUR_DIR)/ext4-root $(EXT4_IMAGE)

# The feature lists are pinned (not left to mke2fs defaults) so the matrix
# shapes stay stable across e2fsprogs versions. The images are .PHONY like
# $(EXT4_IMAGE): rebuilding them is cheap and an interrupted build can never
# leave a stale half-written image behind.
.PHONY: $(EXT_MATRIX_IMAGES)
$(EXT_MATRIX_DIR)/ext2.img:
	@mkdir -p $(EXT_MATRIX_DIR)
	@truncate -s 64M $@
	@mke2fs -q -F -t ext2 -b 4096 -I 256 \
		-O sparse_super,large_file,filetype,resize_inode,dir_index,ext_attr $@

$(EXT_MATRIX_DIR)/ext2_i128.img:
	@mkdir -p $(EXT_MATRIX_DIR)
	@truncate -s 64M $@
	@mke2fs -q -F -t ext2 -b 4096 -I 128 \
		-O sparse_super,large_file,filetype,resize_inode,dir_index,ext_attr $@

$(EXT_MATRIX_DIR)/ext2_i128_nori.img:
	@mkdir -p $(EXT_MATRIX_DIR)
	@truncate -s 64M $@
	@mke2fs -q -F -t ext2 -b 4096 -I 128 \
		-O sparse_super,large_file,filetype,^resize_inode,dir_index,ext_attr $@

$(EXT_MATRIX_DIR)/ext4_journal.img:
	@mkdir -p $(EXT_MATRIX_DIR)
	@truncate -s 64M $@
	@mke2fs -q -F -t ext4 -b 4096 -I 256 \
		-O extent,filetype,has_journal,^metadata_csum,^dir_index,^64bit,^flex_bg,^inline_data,^resize_inode $@

$(EXT_MATRIX_DIR)/ext4_noextents.img:
	@mkdir -p $(EXT_MATRIX_DIR)
	@truncate -s 64M $@
	@mke2fs -q -F -t ext4 -b 4096 -I 256 \
		-O ^extent,filetype,^has_journal,^metadata_csum,^dir_index,^64bit,^flex_bg,^inline_data,^resize_inode $@

$(EXT_MATRIX_DIR)/neg_nofiletype.img:
	@mkdir -p $(EXT_MATRIX_DIR)
	@truncate -s 64M $@
	@mke2fs -q -F -t ext2 -b 4096 -I 256 -O ^filetype $@

$(EXT_MATRIX_DIR)/neg_rev0.img:
	@mkdir -p $(EXT_MATRIX_DIR)
	@truncate -s 64M $@
	@mke2fs -q -F -r 0 -b 4096 $@

$(EXT_MATRIX_DIR)/neg_metadata_csum.img:
	@mkdir -p $(EXT_MATRIX_DIR)
	@truncate -s 64M $@
	@mke2fs -q -F -t ext4 -b 4096 -I 256 \
		-O extent,filetype,metadata_csum,^has_journal,^dir_index,^64bit,^flex_bg,^inline_data,^resize_inode $@

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
	@truncate -s $(XFSTESTS_DISK_SIZE) $(XFSTESTS_TEST_IMAGE)
	@mkfs.ext2 $(XFSTESTS_TEST_IMAGE)

$(XFSTESTS_SCRATCH_IMAGE):
	@mkdir -p $(BUILD_DIR)
	@truncate -s $(XFSTESTS_DISK_SIZE) $(XFSTESTS_SCRATCH_IMAGE)
	@mkfs.ext2 $(XFSTESTS_SCRATCH_IMAGE)

.PHONY: format
format:
	@$(MAKE) --no-print-directory -C src/regression format
	@nixfmt nix

.PHONY: check
check:
	@$(MAKE) --no-print-directory -C src/regression check
	@nixfmt --check nix

.PHONY: clean
clean:
	@rm -rf $(BUILD_DIR)
