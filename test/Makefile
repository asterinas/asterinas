# SPDX-License-Identifier: MPL-2.0

ARCH ?= x86_64
SYSCALL_TEST_WORKDIR ?= /tmp

MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
BUILD_DIR := $(CUR_DIR)/build
INITRAMFS := $(BUILD_DIR)/initramfs
INITRAMFS_FILELIST := $(BUILD_DIR)/initramfs.filelist
INITRAMFS_SKIP_GZIP ?= 0
ifeq ($(INITRAMFS_SKIP_GZIP),1)
INITRAMFS_IMAGE := $(BUILD_DIR)/initramfs.cpio
else
INITRAMFS_IMAGE := $(BUILD_DIR)/initramfs.cpio.gz
endif
EXT2_IMAGE := $(BUILD_DIR)/ext2.img
EXFAT_IMAGE := $(BUILD_DIR)/exfat.img

# Include benchmark, if BENCHMARK is set.
ifeq ($(BENCHMARK), none)
ENABLE_BENCHMARK = false
else
ENABLE_BENCHMARK = true
endif

# Include syscall_test, if the BUILD_SYSCALL_TEST variable is set.
ifeq ($(BUILD_SYSCALL_TEST), 1)
ENABLE_SYSCALL_TEST = true
else
ENABLE_SYSCALL_TEST = false
endif

.PHONY: all
all: build

.PHONY: $(INITRAMFS_IMAGE)
$(INITRAMFS_IMAGE): $(INITRAMFS_FILELIST)
	@if ! cmp -s $(INITRAMFS_FILELIST) $(INITRAMFS_FILELIST).previous || ! test -f $@; then \
		echo "Generating the initramfs image..."; \
		cp -f $(INITRAMFS_FILELIST) $(INITRAMFS_FILELIST).previous; \
		( \
			cd $(INITRAMFS); \
			# `$(INITRAMFS_FILELIST)` contains files' last modification \
			# time in the first column, and files' relative path to \
			# `$(INITRAMFS)` in the second column. This prunes the first \
			# column and passes the second column to `cpio`. \
			cut -d " " -f 2- $(INITRAMFS_FILELIST) | \
				cpio -o -H newc | \
					if [ "$(INITRAMFS_SKIP_GZIP)" != 1 ]; then \
						gzip; \
					else \
						cat; \
					fi \
		) > $@; \
	fi

.PHONY: $(INITRAMFS_FILELIST)
$(INITRAMFS_FILELIST): $(INITRAMFS)
	@(cd $(INITRAMFS); find . -printf "%T@ %p\n") > $(INITRAMFS_FILELIST)

.PHONY: $(INITRAMFS)
$(INITRAMFS):
	@nix-build \
		--argstr target $(ARCH) \
		--argstr enableBenchmark $(ENABLE_BENCHMARK) \
		--argstr enableSyscallTest $(ENABLE_SYSCALL_TEST) \
		--argstr syscallTestWorkDir $(SYSCALL_TEST_WORKDIR) \
		--out-link $@ \
		nix/initramfs.nix
	@chmod -R 0755 $(INITRAMFS)/benchmark
	@chmod -R 0755 $(INITRAMFS)/etc
	@chmod -R 0755 $(INITRAMFS)/opt
	@chmod -R 0755 $(INITRAMFS)/test
	@chmod -R 0755 $(INITRAMFS)/var
	@chmod -R 1777 $(INITRAMFS)/tmp

$(EXT2_IMAGE):
	@dd if=/dev/zero of=$(EXT2_IMAGE) bs=2G count=1
	@mke2fs $(EXT2_IMAGE)

$(EXFAT_IMAGE):
	@fallocate -l 64M $(EXFAT_IMAGE)
	@mkfs.exfat $(EXFAT_IMAGE)

.PHONY: build
build: $(INITRAMFS_IMAGE) $(EXT2_IMAGE) $(EXFAT_IMAGE)

.PHONY: format
format:
	@$(MAKE) --no-print-directory -C apps format
	@nixfmt nix

.PHONY: check
check:
	@$(MAKE) --no-print-directory -C apps check

.PHONY: clean
clean:
	@rm -rf $(BUILD_DIR)
