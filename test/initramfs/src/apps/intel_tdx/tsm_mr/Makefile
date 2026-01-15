# SPDX-License-Identifier: MPL-2.0

CUR_DIR := $(shell dirname $(realpath $(firstword $(MAKEFILE_LIST))))
CUR_DIR_NAME := $(shell basename $(realpath $(CUR_DIR)))
BUILD_DIR := $(CUR_DIR)/../../../../build
OUTPUT_DIR ?= $(BUILD_DIR)/initramfs/test
BIN_OUTPUT_DIR := $(OUTPUT_DIR)/intel_tdx/tsm_mr

.PHONY: all

all: $(BIN_OUTPUT_DIR) $(BIN_OUTPUT_DIR)/tsm_mr.sh

$(BIN_OUTPUT_DIR):
	@mkdir -p $(BIN_OUTPUT_DIR)

$(BIN_OUTPUT_DIR)/tsm_mr.sh: $(CUR_DIR)/tsm_mr.sh
	@cp $< $@
	@chmod +x $@
	@echo "COPY <= $@"
