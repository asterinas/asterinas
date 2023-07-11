MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
BUILD_DIR := $(CUR_DIR)/build
INITRAMFS := $(BUILD_DIR)/initramfs
RAMDISK := $(BUILD_DIR)/ramdisk.cpio.gz
SHELL := /bin/bash

ifneq (, $(wildcard $(INITRAMFS)/. ))
	INITRAMFS_DIRS := $(shell find $(INITRAMFS) -type d 2>/dev/null | sed 's/ /\\ /g' | sed 's/:/\\:/g' || true)
	INITRAMFS_FILES := $(shell find $(INITRAMFS) -type f 2>/dev/null | sed 's/ /\\ /g' | sed 's/:/\\:/g' || true)
endif

.PHONY: all clean

all: build

$(INITRAMFS):
	@rm -rf $@ && mkdir -p $@
	@# Mkdir necessary folders
	@mkdir -p $@/bin $@/etc $@/sbin $@/usr/bin $@/root $@/tmp $@/opt $@/proc $@/dev $@/lib64 $@/lib/x86_64-linux-gnu
	@# Install busybox
	@/bin/busybox --install -s $@/bin
	@cp /usr/bin/busybox $@/usr/bin
	@# Copy necessary libs
	@cp -L /lib64/ld-linux-x86-64.so.2 $@/lib64
	@cp -L /lib/x86_64-linux-gnu/libc.so.6 $@/lib/x86_64-linux-gnu
	@cp -L /lib/x86_64-linux-gnu/libstdc++.so.6 $@/lib/x86_64-linux-gnu
	@cp -L /lib/x86_64-linux-gnu/libm.so.6 $@/lib/x86_64-linux-gnu
	@cp -L /lib/x86_64-linux-gnu/libgcc_s.so.1 $@/lib/x86_64-linux-gnu
	@cp -L /lib/x86_64-linux-gnu/libpthread.so.0 $@/lib/x86_64-linux-gnu
	@# Copy from apps
	@make --no-print-directory -C apps
ifeq ($(ENABLE_SYSCALL_TEST), 1)
	@# Copy syscall test suite
	@make --no-print-directory -C syscall_test
endif

$(RAMDISK): $(INITRAMFS) $(INITRAMFS_DIRS) $(INITRAMFS_FILES)
	@echo "Generating the ramdisk image..."
	@(cd $(INITRAMFS); find . | cpio -o -H newc | gzip) > $@

build: $(RAMDISK)

clean:
	@rm -rf $(BUILD_DIR)
