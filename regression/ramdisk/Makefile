MAKEFLAGS += --no-builtin-rules # Prevent the implicit rules from compiling ".c" or ".s" files automatically.
APPS := ../apps
BUILD_DIR := ./build
INITRAMFS := ./initramfs
RAMDISK := $(BUILD_DIR)/ramdisk.cpio
SHELL := /bin/bash

ifneq (, $(wildcard $(APPS)/. ))
	APPS_DIRS := $(shell find $(APPS) -type d 2>/dev/null | sed 's/ /\\ /g' | sed 's/:/\\:/g' || true)
	APPS_FILES := $(shell find $(APPS) -type f 2>/dev/null | sed 's/ /\\ /g' | sed 's/:/\\:/g' || true)
endif

ifneq (, $(wildcard $(INITRAMFS)/. ))
	INITRAMFS_DIRS := $(shell find $(INITRAMFS) -type d 2>/dev/null | sed 's/ /\\ /g' | sed 's/:/\\:/g' || true)
	INITRAMFS_FILES := $(shell find $(INITRAMFS) -type f 2>/dev/null | sed 's/ /\\ /g' | sed 's/:/\\:/g' || true)
endif


.PHONY: all clean prepare_libs

all: $(RAMDISK)

$(INITRAMFS): $(APPS) $(APPS_DIRS) $(APPS_FILES)
	@rm -rf $@ && mkdir -p $@
# Copy Apps
	@cp -a $(APPS)/* $@
	@cd $@ && find . \( -name "*.s" -o -name "*.c" -o -name "Makefile" -o -name "README.md" \) -delete
# Mkdir folders
	@mkdir -p $@/tmp
	@mkdir -p $@/test
	@mkdir -p $@/lib64
	@mkdir -p $@/lib/x86_64-linux-gnu
# Copy libs
	@cp -L /lib64/ld-linux-x86-64.so.2 $@/lib64
	@cp -L /lib/x86_64-linux-gnu/libc.so.6 $@/lib/x86_64-linux-gnu
	@cp -L /lib/x86_64-linux-gnu/libstdc++.so.6 $@/lib/x86_64-linux-gnu
	@cp -L /lib/x86_64-linux-gnu/libm.so.6 $@/lib/x86_64-linux-gnu
	@cp -L /lib/x86_64-linux-gnu/libgcc_s.so.1 $@/lib/x86_64-linux-gnu
	@cp -L /lib/x86_64-linux-gnu/libpthread.so.0 $@/lib/x86_64-linux-gnu

$(RAMDISK): $(INITRAMFS) $(INITRAMFS_DIRS) $(INITRAMFS_FILES)
	@echo "Generating the ramdisk image..."
	@rm -rf $(BUILD_DIR) && mkdir -p $(BUILD_DIR)
	@./mkinitramfs $(INITRAMFS) $@

clean:
	@rm -rf $(INITRAMFS) $(BUILD_DIR)
