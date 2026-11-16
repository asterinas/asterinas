# SPDX-License-Identifier: MPL-2.0

MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
INITRAMFS ?= $(CUR_DIR)/../../build/initramfs
RUN_BASH := $(CUR_DIR)/run_conformance_test.sh
TARGET_DIR := $(INITRAMFS)/opt
CONFORMANCE_TEST_SUITE ?= ltp
VALID_SUITES := ltp gvisor kselftest pjdfstest xfstests erofs
SUITE_DIR := $(INITRAMFS)/opt/$(CONFORMANCE_TEST_SUITE)
SUITE_SOURCE_DIR := $(CONFORMANCE_TEST_SUITE)
ifeq ($(CONFORMANCE_TEST_SUITE), erofs)
SUITE_SOURCE_DIR := erofs_tests
endif

$(if $(filter $(CONFORMANCE_TEST_SUITE),$(VALID_SUITES)),,\
	$(error CONFORMANCE_TEST_SUITE='$(CONFORMANCE_TEST_SUITE)' not one of: $(VALID_SUITES)))

.PHONY: all
all: $(TARGET_DIR)

$(TARGET_DIR): $(RUN_BASH) $(SUITE_DIR)
	@# Copy bash script
	@cp -f $(RUN_BASH) $@

$(SUITE_DIR):
	@$(MAKE) --no-print-directory -C $(SUITE_SOURCE_DIR)

.PHONY: clean
clean:
	@rm -rf $(TARGET_DIR)
