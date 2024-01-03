# SPDX-License-Identifier: MPL-2.0

MAIN_MAKEFILE := $(firstword $(MAKEFILE_LIST))
INCLUDE_MAKEFILE := $(lastword $(MAKEFILE_LIST))
CUR_DIR := $(shell dirname $(realpath $(MAIN_MAKEFILE)))
CUR_DIR_NAME := $(shell basename $(realpath $(CUR_DIR)))
REGRESSION_BUILD_DIR := $(CUR_DIR)/../../build/initramfs/regression
OBJ_OUTPUT_DIR := $(REGRESSION_BUILD_DIR)/$(CUR_DIR_NAME)
C_SRCS := $(wildcard *.c)
C_OBJS := $(addprefix $(OBJ_OUTPUT_DIR)/,$(C_SRCS:%.c=%))
ASM_SRCS := $(wildcard *.s)
ASM_OBJS := $(addprefix $(OBJ_OUTPUT_DIR)/,$(ASM_SRCS:%.s=%))
CC := gcc
C_FLAGS :=

.PHONY: all

all: $(OBJ_OUTPUT_DIR) $(C_OBJS) $(ASM_OBJS)

$(OBJ_OUTPUT_DIR):
	@mkdir -p $(OBJ_OUTPUT_DIR)

$(CUR_DIR)/../../build/initramfs/regression/$(CUR_DIR_NAME)/%: %.c
	@$(CC) $(C_FLAGS) $(EXTRA_C_FLAGS) $< -o $@
	@echo "CC <= $@"

$(CUR_DIR)/../../build/initramfs/regression/$(CUR_DIR_NAME)/%: %.s
	@$(CC) $(C_FLAGS) $(EXTRA_C_FLAGS) $< -o $@
	@echo "CC <= $@"
