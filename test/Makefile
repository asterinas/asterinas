# SPDX-License-Identifier: MPL-2.0

ARCH ?= x86_64
SMP ?= 1
SYSCALL_TEST_SUITE ?= ltp
SYSCALL_TEST_WORKDIR ?= /tmp
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

.PHONY: all
all: build

.PHONY: build
build: $(INITRAMFS_IMAGE) $(EXT2_IMAGE) $(EXFAT_IMAGE)

.PHONY: $(INITRAMFS_IMAGE)
$(INITRAMFS_IMAGE): $(INITRAMFS)
	@nix-build \
		--argstr target $(ARCH) \
		--arg enableBenchmark $(ENABLE_BENCHMARK) \
		--arg enableSyscallTest $(ENABLE_SYSCALL_TEST) \
		--argstr syscallTestSuite $(SYSCALL_TEST_SUITE) \
		--argstr syscallTestWorkDir $(SYSCALL_TEST_WORKDIR) \
		--arg initramfsCompressed $(INITRAMFS_COMPRESSED) \
		--arg smp $(SMP) \
		--out-link $@ \
		nix -A initramfs-image

.PHONY: $(INITRAMFS)
$(INITRAMFS):
	@nix-build \
		--argstr target $(ARCH) \
		--arg enableBenchmark $(ENABLE_BENCHMARK) \
		--arg enableSyscallTest $(ENABLE_SYSCALL_TEST) \
		--argstr syscallTestSuite $(SYSCALL_TEST_SUITE) \
		--argstr syscallTestWorkDir $(SYSCALL_TEST_WORKDIR) \
		--arg smp $(SMP) \
		--out-link $@ \
		nix -A initramfs

$(EXT2_IMAGE):
	@dd if=/dev/zero of=$(EXT2_IMAGE) bs=2G count=1
	@mke2fs $(EXT2_IMAGE)

$(EXFAT_IMAGE):
	@fallocate -l 64M $(EXFAT_IMAGE)
	@mkfs.exfat $(EXFAT_IMAGE)

.PHONY: format
format:
	@$(MAKE) --no-print-directory -C src/apps format
	@nixfmt nix

.PHONY: check
check:
	@$(MAKE) --no-print-directory -C src/apps check

.PHONY: clean
clean:
	@rm -rf $(BUILD_DIR)
