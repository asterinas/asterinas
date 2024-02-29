# SPDX-License-Identifier: MPL-2.0

MAKEFLAGS += --no-builtin-rules # Prevent the implicit rules from compiling ".c" or ".s" files automatically.
MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
INITRAMFS ?= $(CUR_DIR)/../build/initramfs
REGRESSION_BUILD_DIR ?= $(INITRAMFS)/regression
TEST_APPS := signal_c pthread network hello_world hello_pie hello_c fork_c fork execve pty

.PHONY: all
all: $(TEST_APPS) scripts

.PHONY: $(TEST_APPS)
$(TEST_APPS):
	@make --no-print-directory -C $@

$(REGRESSION_BUILD_DIR):
	@mkdir -p $@

.PHONY: scripts
scripts: | $(REGRESSION_BUILD_DIR)
	@make --no-print-directory BUILD_DIR=$(REGRESSION_BUILD_DIR) -C scripts
