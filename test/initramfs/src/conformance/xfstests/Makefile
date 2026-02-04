# SPDX-License-Identifier: MPL-2.0

MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
INITRAMFS ?= $(CUR_DIR)/../../build/initramfs
XFSTESTS_PREBUILT_DIR ?= /tmp
XFSTESTS_DIR := $(INITRAMFS)/opt/xfstests

.PHONY: all
all: $(XFSTESTS_DIR)

$(XFSTESTS_DIR):
	@mkdir -p $@/{test,scratch}
	@cp -r $(XFSTESTS_PREBUILT_DIR)/xfstests/* $@/
