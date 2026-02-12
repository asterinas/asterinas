# SPDX-License-Identifier: MPL-2.0

MAIN_MAKEFILE := $(firstword $(MAKEFILE_LIST))
INCLUDE_MAKEFILE := $(lastword $(MAKEFILE_LIST))
CUR_DIR := $(shell dirname $(realpath $(MAIN_MAKEFILE)))
INCLUDE_MAKEFILE_DIR := $(shell dirname $(realpath $(INCLUDE_MAKEFILE)))
COMMON_PREFIX := $(shell dirname $(INCLUDE_MAKEFILE_DIR))
CUR_DIR_NAME := $(subst $(COMMON_PREFIX)/,,$(CUR_DIR))

BUILD_DIR ?= $(INCLUDE_MAKEFILE_DIR)/../../../build
OUTPUT_DIR ?= $(BUILD_DIR)/initramfs/test
OBJ_OUTPUT_DIR := $(OUTPUT_DIR)/$(CUR_DIR_NAME)
DEP_OUTPUT_DIR := $(BUILD_DIR)/dep/$(CUR_DIR_NAME)
C_SRCS := $(wildcard *.c)
C_OBJS := $(addprefix $(OBJ_OUTPUT_DIR)/,$(C_SRCS:%.c=%))
C_DEPS := $(addprefix $(DEP_OUTPUT_DIR)/,$(C_SRCS:%.c=%.d))
ASM_SRCS := $(wildcard *.S)
ASM_OBJS := $(addprefix $(OBJ_OUTPUT_DIR)/,$(ASM_SRCS:%.S=%))
SCRIPTS := $(addprefix $(OBJ_OUTPUT_DIR)/, $(wildcard *.sh))

CC ?= gcc
C_FLAGS += -Wall -Werror

HOST_PLATFORM ?= x86_64-linux
TEST_PLATFORM ?= asterinas

ifeq ($(TEST_PLATFORM),asterinas)
C_FLAGS += -D__asterinas__
endif

ifneq ($(C_OBJS_FILTER),)
    FILTERED_C_OBJS := $(addprefix $(OBJ_OUTPUT_DIR)/,$(C_OBJS_FILTER))
    C_OBJS := $(filter-out $(FILTERED_C_OBJS),$(C_OBJS))
endif

ifneq ($(ASM_OBJS_FILTER),)
    FILTERED_ASM_OBJS := $(addprefix $(OBJ_OUTPUT_DIR)/,$(ASM_OBJS_FILTER))
    ASM_OBJS := $(filter-out $(FILTERED_ASM_OBJS),$(ASM_OBJS))
endif

ifneq ($(SCRIPTS_FILTER),)
    FILTERED_SCRIPTS := $(addprefix $(OBJ_OUTPUT_DIR)/,$(SCRIPTS_FILTER))
    SCRIPTS := $(filter-out $(FILTERED_SCRIPTS),$(SCRIPTS))
endif

ifneq ($(SUBDIRS_FILTER),)
    SUBDIRS := $(filter-out $(SUBDIRS_FILTER),$(SUBDIRS))
endif

.PHONY: all
all: $(C_OBJS) $(ASM_OBJS) $(SUBDIRS) $(SCRIPTS)

$(OBJ_OUTPUT_DIR) $(DEP_OUTPUT_DIR):
	@mkdir -p $@

$(OBJ_OUTPUT_DIR)/%: %.c | $(OBJ_OUTPUT_DIR) $(DEP_OUTPUT_DIR)
	@$(CC) $(C_FLAGS) $< -o $@ $(EXTRA_C_FLAGS) \
		-MMD -MF $(DEP_OUTPUT_DIR)/$*.d
	@echo "CC <= $@"

-include $(C_DEPS)

$(OBJ_OUTPUT_DIR)/%: %.S | $(OBJ_OUTPUT_DIR)
	@$(CC) $(C_FLAGS) $(EXTRA_C_FLAGS) $< -o $@
	@echo "CC <= $@"

$(OBJ_OUTPUT_DIR)/%.sh: %.sh | $(OBJ_OUTPUT_DIR)
	@cp $< $@

.PHONY: $(SUBDIRS)
$(SUBDIRS):
	@make --no-print-directory -C $@
