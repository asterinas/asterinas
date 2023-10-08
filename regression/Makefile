MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
BUILD_DIR := $(CUR_DIR)/build
INITRAMFS := $(BUILD_DIR)/initramfs
INITRAMFS_IMAGE := $(BUILD_DIR)/initramfs.cpio.gz
SHELL := /bin/bash
INITRAMFS_EMPTY_DIRS := \
	$(INITRAMFS)/etc \
	$(INITRAMFS)/sbin \
	$(INITRAMFS)/root \
	$(INITRAMFS)/tmp \
	$(INITRAMFS)/opt \
	$(INITRAMFS)/proc \
	$(INITRAMFS)/dev
INITRAMFS_ALL_DIRS := \
	$(INITRAMFS)/lib/x86_64-linux-gnu \
	$(INITRAMFS)/lib64 \
	$(INITRAMFS)/bin \
	$(INITRAMFS)/usr/bin \
	$(INITRAMFS)/usr/sbin \
	$(INITRAMFS)/regression \
	$(INITRAMFS_EMPTY_DIRS)

.PHONY: all clean

all: build

$(INITRAMFS)/lib/x86_64-linux-gnu:
	@mkdir -p $@
	@cp -L /lib/x86_64-linux-gnu/libc.so.6 $@
	@cp -L /lib/x86_64-linux-gnu/libstdc++.so.6 $@
	@cp -L /lib/x86_64-linux-gnu/libm.so.6 $@
	@cp -L /lib/x86_64-linux-gnu/libgcc_s.so.1 $@
	@cp -L /lib/x86_64-linux-gnu/libpthread.so.0 $@

$(INITRAMFS)/lib64:
	@mkdir -p $@
	@cp -L /lib64/ld-linux-x86-64.so.2 $@

# Install busybox into /bin and /usr/bin.
$(INITRAMFS)/bin:
	@mkdir -p $@
	@/bin/busybox --install -s $@

$(INITRAMFS)/usr/bin: $(INITRAMFS)/bin
	@mkdir -p $@
	@cp /usr/bin/busybox $@

# Copy nologin for user nobody.
$(INITRAMFS)/usr/sbin: 
	@mkdir -p $@
	@cp /usr/sbin/nologin $@

# Copy from apps.
$(INITRAMFS)/regression:
	@make --no-print-directory -C apps

# Make necessary directories.
$(INITRAMFS_EMPTY_DIRS):
	@mkdir -p $@

$(INITRAMFS)/opt/syscall_test:
	@make --no-print-directory -C syscall_test

# If the BUILD_SYSCALL_TEST variable is set, we should depend on the
# sub make output to do incremental building.
ifeq ($(BUILD_SYSCALL_TEST), 1)
$(INITRAMFS_IMAGE): $(INITRAMFS_ALL_DIRS) $(INITRAMFS)/opt/syscall_test
else
$(INITRAMFS_IMAGE): $(INITRAMFS_ALL_DIRS)
endif
	@echo "Generating the initramfs image..."
	@(cd $(INITRAMFS); find . | cpio -o -H newc | gzip) > $@

build: $(INITRAMFS_IMAGE)

clean:
	@rm -rf $(BUILD_DIR)
