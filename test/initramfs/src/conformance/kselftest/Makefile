# SPDX-License-Identifier: MPL-2.0

KSELFTEST_PREBUILT_DIR ?= /opt/kselftest
MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
INITRAMFS ?= $(CUR_DIR)/../../../build/initramfs
TARGET_DIR := $(INITRAMFS)/opt/kselftest
RUN_BASH := $(CUR_DIR)/run_kselftest.sh
BLOCK_LIST := $(CUR_DIR)/blocklists
EXT2_BLOCK_LIST := $(CUR_DIR)/blocklists.ext2
EXFAT_BLOCK_LIST := $(CUR_DIR)/blocklists.exfat

$(if $(wildcard $(BLOCK_LIST)),,$(error missing default blocklist: $(BLOCK_LIST)))

.PHONY: all
all: $(TARGET_DIR)

$(TARGET_DIR): $(RUN_BASH) $(BLOCK_LIST) $(EXT2_BLOCK_LIST) $(EXFAT_BLOCK_LIST)
	@rm -rf $@ && mkdir -p $@
	@# Copy testcases
	@cp -rf $(KSELFTEST_PREBUILT_DIR)/* $@
	@# Copy blocklists
	@cp -f $(BLOCK_LIST) $@/blocklists
	@cp -f $(EXT2_BLOCK_LIST) $@/blocklists.ext2
	@cp -f $(EXFAT_BLOCK_LIST) $@/blocklists.exfat
	@# Copy bash script
	@cp -f $(RUN_BASH) $@

.PHONY: clean
clean:
	@rm -rf $(TARGET_DIR)
