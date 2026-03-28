# SPDX-License-Identifier: MPL-2.0

KSELFTEST_PREBUILT_DIR ?= /opt/kselftest
MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
INITRAMFS ?= $(CUR_DIR)/../../../build/initramfs
TARGET_DIR := $(INITRAMFS)/opt/kselftest
RUN_BASH := $(CUR_DIR)/run_kselftest.sh
BLOCK_LIST := $(CUR_DIR)/blocklists

.PHONY: all
all: $(TARGET_DIR)

$(TARGET_DIR): $(RUN_BASH) $(BLOCK_LIST)
	@rm -rf $@ && mkdir -p $@
	@# Copy testcases
	@cp -rf $(KSELFTEST_PREBUILT_DIR)/* $@
	@# Copy blocklists
	@cp -rf $(BLOCK_LIST) $@
	@# Copy bash script
	@cp -f $(RUN_BASH) $@

.PHONY: clean
clean:
	@rm -rf $(TARGET_DIR)
