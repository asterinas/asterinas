# SPDX-License-Identifier: MPL-2.0

MAKEFLAGS += --no-builtin-rules # Prevent the implicit rules from compiling ".c" or ".s" files automatically.
MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
INITRAMFS ?= $(CUR_DIR)/../build/initramfs
REGRESSION_BUILD_DIR ?= $(INITRAMFS)/regression
TEST_APPS := signal_c pthread network hello_world hello_pie hello_c fork_c fork execve pty

.PHONY: all

all: 
	@mkdir -p $(REGRESSION_BUILD_DIR)
	@for test_app in $(TEST_APPS); \
		do make --no-print-directory -C $${test_app}; \
	done
	@make --no-print-directory BUILD_DIR=$(REGRESSION_BUILD_DIR) -C scripts
