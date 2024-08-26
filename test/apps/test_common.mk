# SPDX-License-Identifier: MPL-2.0

MAIN_MAKEFILE := $(firstword $(MAKEFILE_LIST))
CUR_DIR := $(shell dirname $(realpath $(MAIN_MAKEFILE)))
CUR_DIR_NAME := $(shell basename $(realpath $(CUR_DIR)))
TEST_BUILD_DIR := $(BUILD_DIR)/$(CUR_DIR_NAME)
TEST_INSTALL_DIR := $(INSTALL_DIR)/$(CUR_DIR_NAME)
DEP_OUTPUT_DIR := $(TEST_BUILD_DIR)/dep
C_SRCS := $(wildcard *.c)
C_OBJS := $(addprefix $(TEST_INSTALL_DIR)/,$(C_SRCS:%.c=%))
C_DEPS := $(addprefix $(DEP_OUTPUT_DIR)/,$(C_SRCS:%.c=%.d))
ASM_SRCS := $(wildcard *.S)
ASM_OBJS := $(addprefix $(TEST_INSTALL_DIR)/,$(ASM_SRCS:%.S=%))

CC := gcc
C_FLAGS := -Wall -Werror

.PHONY: all
all: $(C_OBJS) $(ASM_OBJS)

$(TEST_INSTALL_DIR) $(TEST_BUILD_DIR) $(DEP_OUTPUT_DIR):
	@mkdir -p $@

$(TEST_INSTALL_DIR)/%: %.c | $(TEST_INSTALL_DIR) $(DEP_OUTPUT_DIR)
	@$(CC) $(C_FLAGS) $(EXTRA_C_FLAGS) $< -o $@ \
		-MMD -MF $(DEP_OUTPUT_DIR)/$*.d
	@echo "CC <= $@"

-include $(C_DEPS)

$(TEST_INSTALL_DIR)/%: %.S | $(TEST_INSTALL_DIR)
	@$(CC) $(C_FLAGS) $(EXTRA_C_FLAGS) $< -o $@
	@echo "CC <= $@"
