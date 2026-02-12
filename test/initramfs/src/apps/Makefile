# SPDX-License-Identifier: MPL-2.0

HOST_PLATFORM ?= x86_64-linux

# Prevent the implicit rules from compiling ".c" or ".s" files automatically.
MAKEFLAGS += --no-builtin-rules

MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
BUILD_DIR ?= $(CUR_DIR)/../../build
OUTPUT_DIR ?= $(BUILD_DIR)/initramfs/test

# REMINDER: Whenever you change this list, update the corresponding `default.nix` file.
SUBDIRS := \
	device \
	fs \
	hello_world \
	io \
	ipc \
	memory \
	network \
	process \
	scripts \
	security \

ifeq ($(HOST_PLATFORM), x86_64-linux)
SUBDIRS += intel_tdx
endif

# The C head and source files of all the apps, excluding the downloaded mongoose files
C_SOURCES := $(shell find . -type f \( -name "*.c" -or -name "*.h" \))

.PHONY: all
all: $(SUBDIRS)

$(OUTPUT_DIR):
	@mkdir -p $@

.PHONY: $(SUBDIRS)
$(SUBDIRS): $(OUTPUT_DIR)
	@make --no-print-directory BUILD_DIR=$(BUILD_DIR) OUTPUT_DIR=$(OUTPUT_DIR) -C $@

.PHONY: format
format:
	@echo "Fixing code format for general tests..."
	@clang-format -i $(C_SOURCES)

.PHONY: check
check:
	@echo "Checking code format for general tests..."
	@clang-format --dry-run --Werror $(C_SOURCES)
