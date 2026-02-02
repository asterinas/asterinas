# SPDX-License-Identifier: MPL-2.0

MAIN_MAKEFILE := $(firstword $(MAKEFILE_LIST))
CUR_DIR := $(shell dirname $(realpath $(MAIN_MAKEFILE)))
CUR_DIR_NAME := $(shell basename $(realpath $(CUR_DIR)))
BUILD_DIR := $(CUR_DIR)/../../../build
OBJ_OUTPUT_DIR := $(BUILD_DIR)/initramfs/test/$(CUR_DIR_NAME)

.PHONY: all

all: $(OBJ_OUTPUT_DIR) $(OBJ_OUTPUT_DIR)/tsm_mr.sh

$(OBJ_OUTPUT_DIR):
	@mkdir -p $(OBJ_OUTPUT_DIR)

$(OBJ_OUTPUT_DIR)/tsm_mr.sh: $(CUR_DIR)/tsm_mr.sh
	@cp $< $@
	@chmod +x $@
	@echo "COPY <= $@"
