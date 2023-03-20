MAKEFLAGS += --no-builtin-rules # Prevent the implicit rules from compiling ".c" or ".s" files automatically.
APPS := ../test/apps
LTP_TEST := ../test/ltp/ltp_test
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

.PHONY: all clean

all: $(RAMDISK)

$(INITRAMFS): $(APPS) $(APPS_DIRS) $(APPS_FILES)
	@rm -rf $@ && mkdir -p $@
	@mkdir -p $@/test
	@mkdir -p $@/bin
	@cp -a $(APPS)/* $@/test
	@cp -a $(APPS)/busybox/busybox $@/bin/busybox
	
	@if [ -d "$(LTP_TEST)" ]; then	cp -ar $(LTP_TEST) $@/test; mv $@/test/$(shell basename $(LTP_TEST)) $@/test/ltp; fi
	
	@ln -sf /bin/busybox $@/bin/sh
	@cd $@/test && rm -r busybox
	@cd $@ && find . \( -name "*.s" -o -name "*.c" -o -name "Makefile" -o -name "README.md" \) -delete

$(RAMDISK): $(INITRAMFS) $(INITRAMFS_DIRS) $(INITRAMFS_FILES)
	@echo "Generating the ramdisk image..."
	@rm -rf $(BUILD_DIR) && mkdir -p $(BUILD_DIR)
	@./mkinitramfs $(INITRAMFS) $@
clean:
	@rm -rf $(INITRAMFS) $(BUILD_DIR)
