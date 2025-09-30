# SPDX-License-Identifier: MPL-2.0

MAIN_MAKEFILE := $(firstword $(MAKEFILE_LIST))
INCLUDE_MAKEFILE := $(lastword $(MAKEFILE_LIST))
CUR_DIR := $(shell dirname $(realpath $(MAIN_MAKEFILE)))
CUR_DIR_NAME := $(shell basename $(realpath $(CUR_DIR)))
TDX_ATTEST_DIR ?= $(CUR_DIR)
TDX_SRCS := test_tdx_attest.c tdx_attest.c qgs_msg_lib.cpp
BUILD_DIR := $(CUR_DIR)/../../../build
OBJ_OUTPUT_DIR := $(BUILD_DIR)/initramfs/test/$(CUR_DIR_NAME)
CC ?= gcc
C_FLAGS ?= -Wall -Werror
DCAP_VERSION := DCAP_1.23
DCAP_URL_PREFIX := "https://github.com/intel/SGXDataCenterAttestationPrimitives/raw/$(DCAP_VERSION)/QuoteGeneration/quote_wrapper"
ATOMIC_WGET := $(CUR_DIR)/../../../../tools/atomic_wget.sh

.PHONY: all

all: $(OBJ_OUTPUT_DIR) $(OBJ_OUTPUT_DIR)/generate_tdx_quote

$(OBJ_OUTPUT_DIR):
	@mkdir -p $(OBJ_OUTPUT_DIR)

$(OBJ_OUTPUT_DIR)/generate_tdx_quote: $(addprefix $(TDX_ATTEST_DIR)/,$(TDX_SRCS))
	@$(CC) $(C_FLAGS) $^ -o $@
	@echo "CC <= $@"

$(TDX_ATTEST_DIR)/tdx_attest.c: $(TDX_ATTEST_DIR)/tdx_attest.h $(TDX_ATTEST_DIR)/qgs_msg_lib.h
	@$(ATOMIC_WGET) $@ "$(DCAP_URL_PREFIX)/tdx_attest/tdx_attest.c"

$(TDX_ATTEST_DIR)/tdx_attest.h:
	@$(ATOMIC_WGET) $@ "$(DCAP_URL_PREFIX)/tdx_attest/tdx_attest.h"

$(TDX_ATTEST_DIR)/test_tdx_attest.c:
	@$(ATOMIC_WGET) $@ "$(DCAP_URL_PREFIX)/tdx_attest/test_tdx_attest.c"

$(TDX_ATTEST_DIR)/qgs_msg_lib.cpp:
	@$(ATOMIC_WGET) $@ "$(DCAP_URL_PREFIX)/qgs_msg_lib/qgs_msg_lib.cpp"

$(TDX_ATTEST_DIR)/qgs_msg_lib.h:
	@$(ATOMIC_WGET) $@ "$(DCAP_URL_PREFIX)/qgs_msg_lib/inc/qgs_msg_lib.h"
