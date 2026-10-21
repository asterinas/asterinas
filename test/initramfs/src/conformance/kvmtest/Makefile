# SPDX-License-Identifier: MPL-2.0

KVMTEST_PREBUILT_DIR ?= /opt/kvmtest
MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
INITRAMFS ?= $(CUR_DIR)/../../../build/initramfs
TARGET_DIR := $(INITRAMFS)/opt/kvmtest
RUN_BASH := $(CUR_DIR)/run_kvm_hello_world.sh
TEST_BIN := $(KVMTEST_PREBUILT_DIR)/bin/kvm-hello-world

.PHONY: all
all: $(TARGET_DIR)

$(TARGET_DIR): $(RUN_BASH) $(TEST_BIN)
	@mkdir -p $@
	@install -m 755 $(TEST_BIN) $(RUN_BASH) $@/

.PHONY: clean
clean:
	@rm -rf $(TARGET_DIR)
