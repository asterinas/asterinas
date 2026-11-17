# SPDX-License-Identifier: MPL-2.0

MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
INITRAMFS ?= $(CUR_DIR)/../../../build/initramfs
EROFS_PREBUILT_DIR ?= /tmp
EROFS_DIR := $(INITRAMFS)/opt/erofs

all: $(EROFS_DIR)

$(EROFS_DIR):
	@mkdir -p $@
	@cp -rL $(EROFS_PREBUILT_DIR)/erofs/* $@/

.PHONY: clean
clean:
	@rm -rf $(EROFS_DIR)
