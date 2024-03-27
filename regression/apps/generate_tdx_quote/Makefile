# SPDX-License-Identifier: MPL-2.0

MAIN_MAKEFILE := $(firstword $(MAKEFILE_LIST))
INCLUDE_MAKEFILE := $(lastword $(MAKEFILE_LIST))
CUR_DIR := $(shell dirname $(realpath $(MAIN_MAKEFILE)))
CUR_DIR_NAME := $(shell basename $(realpath $(CUR_DIR)))
REGRESSION_BUILD_DIR := $(CUR_DIR)/../../build/initramfs/regression
OBJ_OUTPUT_DIR := $(REGRESSION_BUILD_DIR)/$(CUR_DIR_NAME)
C_SRCS := $(wildcard *.c)
C_OBJS := $(addprefix $(OBJ_OUTPUT_DIR)/,$(C_SRCS:%.c=%))
CC := gcc
C_FLAGS :=
DCAP_VERSION := DCAP_1.20
DCAP_URL_PREFIX := "https://github.com/intel/SGXDataCenterAttestationPrimitives/raw/$(DCAP_VERSION)/QuoteGeneration/quote_wrapper"

.PHONY: all

all: dcap $(OBJ_OUTPUT_DIR) $(OBJ_OUTPUT_DIR)/generate_tdx_quote

# Download Intel(R) Software Guard Extensions Data Center Attestation Primitives (DCAP) dependencies.
dcap:
	@wget -q -nc \
	"$(DCAP_URL_PREFIX)/tdx_attest/tdx_attest.c" \
	"$(DCAP_URL_PREFIX)/tdx_attest/tdx_attest.h" \
	"$(DCAP_URL_PREFIX)/tdx_attest/test_tdx_attest.c" \
	"$(DCAP_URL_PREFIX)/qgs_msg_lib/qgs_msg_lib.cpp" \
	"$(DCAP_URL_PREFIX)/qgs_msg_lib/inc/qgs_msg_lib.h"

$(OBJ_OUTPUT_DIR):
	@mkdir -p $(OBJ_OUTPUT_DIR)

$(OBJ_OUTPUT_DIR)/generate_tdx_quote: test_tdx_attest.c tdx_attest.c qgs_msg_lib.cpp
	@$(CC) $(C_FLAGS) $^ -o $@
	@echo "CC <= $@"

$(OBJ_OUTPUT_DIR)/%: %.c
	@$(CC) $(C_FLAGS) $< -o $@
	@echo "CC <= $@"
