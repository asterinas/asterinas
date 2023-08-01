MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
BUILD_DIR := $(CUR_DIR)/build
INITRAMFS := $(BUILD_DIR)/initramfs
RAMDISK := $(BUILD_DIR)/ramdisk.cpio.gz
SHELL := /bin/bash

.PHONY: all clean

all: build

TARGETS=\
	$(INITRAMFS)/lib/x86_64-linux-gnu \
	$(INITRAMFS)/lib64 \
	$(INITRAMFS)/bin \
	$(INITRAMFS)/usr/bin \
	$(INITRAMFS)/regression \
	$(INITRAMFS)/etc \
	$(INITRAMFS)/sbin \
	$(INITRAMFS)/root \
	$(INITRAMFS)/tmp \
	$(INITRAMFS)/opt \
	$(INITRAMFS)/proc \
	$(INITRAMFS)/dev

# Copy necessary libs
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

# Install busybox
$(INITRAMFS)/bin:
	@mkdir -p $@
	@/bin/busybox --install -s $@

$(INITRAMFS)/usr/bin: $(INITRAMFS)/bin
	@mkdir -p $@
	@cp /usr/bin/busybox $@

# Copy from apps
$(INITRAMFS)/regression:
	@make --no-print-directory -C apps

# Make necessary directories
$(INITRAMFS)/etc:
	@mkdir -p $@

$(INITRAMFS)/sbin:
	@mkdir -p $@

$(INITRAMFS)/root:
	@mkdir -p $@

$(INITRAMFS)/tmp:
	@mkdir -p $@

$(INITRAMFS)/opt:
	@mkdir -p $@

$(INITRAMFS)/proc:
	@mkdir -p $@

$(INITRAMFS)/dev:
	@mkdir -p $@

$(RAMDISK): $(TARGETS)
	@echo "Generating the ramdisk image..."
	@(cd $(INITRAMFS); find . | cpio -o -H newc | gzip) > $@

build: $(RAMDISK)

clean:
	@rm -rf $(BUILD_DIR)
