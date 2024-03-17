# SPDX-License-Identifier: MPL-2.0

MAIN_MAKEFILE := $(firstword $(MAKEFILE_LIST))
INCLUDE_MAKEFILE := $(lastword $(MAKEFILE_LIST))
CUR_DIR := $(shell dirname $(realpath $(MAIN_MAKEFILE)))
CUR_DIR_NAME := $(shell basename $(realpath $(CUR_DIR)))
BUILD_DIR := $(CUR_DIR)/../../build
OBJ_OUTPUT_DIR := $(BUILD_DIR)/initramfs/regression/$(CUR_DIR_NAME)
DEP_OUTPUT_DIR := $(BUILD_DIR)/dep/$(CUR_DIR_NAME)
C_SRCS := $(wildcard *.c)
C_OBJS := $(addprefix $(OBJ_OUTPUT_DIR)/,$(C_SRCS:%.c=%))
C_DEPS := $(addprefix $(DEP_OUTPUT_DIR)/,$(C_SRCS:%.c=%.d))
ASM_SRCS := $(wildcard *.s)
ASM_OBJS := $(addprefix $(OBJ_OUTPUT_DIR)/,$(ASM_SRCS:%.s=%))
CC := gcc
C_FLAGS := -Wall -Werror

.PHONY: all
all: $(C_OBJS) $(ASM_OBJS)

$(OBJ_OUTPUT_DIR) $(DEP_OUTPUT_DIR):
	@mkdir -p $@

$(OBJ_OUTPUT_DIR)/%: %.c | $(OBJ_OUTPUT_DIR) $(DEP_OUTPUT_DIR)
	@$(CC) $(C_FLAGS) $(EXTRA_C_FLAGS) $< -o $@ \
		-MMD -MF $(DEP_OUTPUT_DIR)/$*.d
	@echo "CC <= $@"

-include $(C_DEPS)

$(OBJ_OUTPUT_DIR)/%: %.s | $(OBJ_OUTPUT_DIR)
	@$(CC) $(C_FLAGS) $(EXTRA_C_FLAGS) $< -o $@
	@echo "CC <= $@"
