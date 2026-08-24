# SPDX-License-Identifier: MPL-2.0

LTP_PREBUILT_DIR ?= /opt/ltp
MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
INITRAMFS ?= $(CUR_DIR)/../../../build/initramfs
TARGET_DIR := $(INITRAMFS)/opt/ltp
RUN_BASH := $(CUR_DIR)/run_ltp_test.sh
BUILD_LTP_PACKAGE := $(CUR_DIR)/build_ltp_package.sh
ALL_TESTS := $(CUR_DIR)/testcases/all.txt
EXT2_BLOCKLIST := $(CUR_DIR)/testcases/blocked/ext2.txt
EXFAT_BLOCKLIST := $(CUR_DIR)/testcases/blocked/exfat.txt

.PHONY: all
all: $(TARGET_DIR)

$(TARGET_DIR): $(BUILD_LTP_PACKAGE) $(RUN_BASH) $(ALL_TESTS) $(EXT2_BLOCKLIST) $(EXFAT_BLOCKLIST)
	@"$(BUILD_LTP_PACKAGE)" "$@"

.PHONY: clean
clean:
	@rm -rf $(TARGET_DIR)
