# SPDX-License-Identifier: MPL-2.0

PJDFSTEST_PREBUILT_DIR ?= /tmp
MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
INITRAMFS ?= $(CUR_DIR)/../../../build/initramfs
PJDFSTEST_DIR := $(INITRAMFS)/opt/pjdfstest

.PHONY: all
all: $(PJDFSTEST_DIR)

$(PJDFSTEST_DIR):
	@mkdir -p $@
	@cp -r $(PJDFSTEST_PREBUILT_DIR)/* $@/

.PHONY: clean
clean:
	@rm -rf $(PJDFSTEST_DIR)
