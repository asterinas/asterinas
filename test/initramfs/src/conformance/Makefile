# SPDX-License-Identifier: MPL-2.0

MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
INITRAMFS ?= $(CUR_DIR)/../../build/initramfs
RUN_BASH := $(CUR_DIR)/run_conformance_test.sh
TARGET_DIR := $(INITRAMFS)/opt
CONFORMANCE_TEST_SUITE ?= ltp
GVISOR_DIR := $(INITRAMFS)/opt/gvisor
LTP_DIR := $(INITRAMFS)/opt/ltp

.PHONY: all
all: $(TARGET_DIR)

$(TARGET_DIR): $(RUN_BASH) $(GVISOR_DIR) $(LTP_DIR)
	@# Copy bash script
	@cp -f $(RUN_BASH) $@

$(GVISOR_DIR):
ifeq ($(CONFORMANCE_TEST_SUITE), gvisor)
	@$(MAKE) --no-print-directory -C gvisor
endif

$(LTP_DIR):
ifeq ($(CONFORMANCE_TEST_SUITE), ltp)
	@$(MAKE) --no-print-directory -C ltp
endif

.PHONY: clean
clean:
	@rm -rf $(TARGET_DIR)
