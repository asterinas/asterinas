# SPDX-License-Identifier: MPL-2.0

NIXOS_TEST_SUITE ?=
NIXOS_TEST_CASE ?=
NIXOS_TEST_TIMEOUT ?= 5min

# Generated configuration file name
TEST_CONFIG_FILE_NAME := _configuration_for_test.nix

# Paths
MAKEFILE_DIR := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))
ASTERINAS_ROOT := $(MAKEFILE_DIR)/../..
NIXOS_CONFIG_DIR := $(ASTERINAS_ROOT)/distro/etc_nixos
BASE_CONFIG := $(NIXOS_CONFIG_DIR)/configuration.nix
TEST_CONFIG := $(NIXOS_CONFIG_DIR)/$(TEST_CONFIG_FILE_NAME)
MERGE_SCRIPT := $(ASTERINAS_ROOT)/test/nixos/common/merge_nixos_config.sh

# NixOS build and run scripts
BUILD_ISO_SCRIPT := $(ASTERINAS_ROOT)/tools/nixos/build_iso.sh
BUILD_NIXOS_SCRIPT := $(ASTERINAS_ROOT)/tools/nixos/build_nixos.sh
RUN_SCRIPT := $(ASTERINAS_ROOT)/tools/nixos/run.sh

# Test directory structure
TEST_DIR := tests/$(NIXOS_TEST_SUITE)
EXTRA_CONFIG := $(TEST_DIR)/extra_config.nix

.PHONY: check-test-name
check-test-name:
	@if [ -z "$(NIXOS_TEST_SUITE)" ]; then \
        echo "Error: NIXOS_TEST_SUITE is not set"; \
        echo "Usage: make [target] NIXOS_TEST_SUITE=<test_name>"; \
        exit 1; \
    fi
	@if [ ! -d "$(TEST_DIR)" ]; then \
        echo "Error: Test directory '$(TEST_DIR)' does not exist"; \
        exit 1; \
    fi

.PHONY: prepare
prepare: check-test-name
	@echo "==> Preparing configuration for test '$(NIXOS_TEST_SUITE)'..."
	@if [ -f "$(EXTRA_CONFIG)" ]; then \
        echo "    Found extra config: $(EXTRA_CONFIG)"; \
        echo "    Merging configurations..."; \
        bash "$(MERGE_SCRIPT)" "$(BASE_CONFIG)" "$(EXTRA_CONFIG)" "$(TEST_CONFIG)"; \
        if [ $$? -ne 0 ]; then \
            echo "Error: Configuration merge failed"; \
            rm "$(TEST_CONFIG)"; \
            exit 1; \
        fi; \
    else \
        echo "    No extra config found, using base configuration"; \
        cp "$(BASE_CONFIG)" "$(TEST_CONFIG)"; \
    fi

.PHONY: iso
iso: prepare
	@echo "==> Building ISO installer for test '$(NIXOS_TEST_NAME)'..."
	@echo "    Building ISO installer...";
	@bash "$(BUILD_ISO_SCRIPT)" "$(TEST_CONFIG_FILE_NAME)";
	@if [ $$? -ne 0 ]; then \
        echo "Error: ISO build failed"; \
        rm "$(TEST_CONFIG)"; \
        exit 1; \
    fi
	@rm "$(TEST_CONFIG)";

.PHONY: nixos
nixos: prepare
	@echo "==> Building NixOS image for test '$(NIXOS_TEST_NAME)'..."
	@bash "$(BUILD_NIXOS_SCRIPT)" "$(TEST_CONFIG_FILE_NAME)";
	@if [ $$? -ne 0 ]; then \
        echo "Error: NixOS build failed"; \
        rm "$(TEST_CONFIG)"; \
        exit 1; \
    fi
	@rm "$(TEST_CONFIG)";

.PHONY: run_nixos
run_nixos: check-test-name
	@cd "$(TEST_DIR)" && \
        TEST_CASE_ARG=""; \
        if [ -n "$(NIXOS_TEST_CASE)" ]; then \
            TEST_CASE_ARG="--test $(NIXOS_TEST_CASE)"; \
        fi; \
        QEMU_CMD="bash $(RUN_SCRIPT) nixos"; \
        cargo run -- --qemu-cmd "$$QEMU_CMD" $$TEST_CASE_ARG || \
        exit 1

.PHONY: check
check:
	@cargo clippy -- -D warnings

.PHONY: format
format:
	@cargo fmt
	@nixfmt .