# SPDX-License-Identifier: MPL-2.0

CUR_DIR := $(shell dirname $(realpath $(firstword $(MAKEFILE_LIST))))
ATOMIC_WGET := $(CUR_DIR)/../../../../../../tools/atomic_wget.sh
BUILD_DIR ?= $(CUR_DIR)/../../../../build
OUTPUT_DIR ?= $(BUILD_DIR)/initramfs/test
BIN_OUTPUT_DIR := $(OUTPUT_DIR)/intel_tdx/gen_quote
TDX_ATTEST_DIR ?= $(CUR_DIR)
TDX_SRCS := test_tdx_attest.c tdx_attest.c qgs_msg_lib.cpp
CC ?= gcc
# FIXME: Disable use-after-free warning as error due to potential false positive
# in realloc/free pattern in `tdx_attest.c`.
C_FLAGS += -Wno-error
DCAP_VERSION := DCAP_1.23
DCAP_URL_PREFIX := "https://github.com/intel/SGXDataCenterAttestationPrimitives/raw/$(DCAP_VERSION)/QuoteGeneration/quote_wrapper"

.PHONY: all
all: $(BIN_OUTPUT_DIR)/gen_quote

$(BIN_OUTPUT_DIR):
	@mkdir -p $@

$(BIN_OUTPUT_DIR)/gen_quote: $(addprefix $(TDX_ATTEST_DIR)/,$(TDX_SRCS)) | $(BIN_OUTPUT_DIR)
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
