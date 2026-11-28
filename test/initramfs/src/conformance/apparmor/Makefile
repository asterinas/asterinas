# SPDX-License-Identifier: MPL-2.0

INITRAMFS ?= ../../build/initramfs
APPARMOR_PREBUILT_DIR ?= /opt/apparmor
TARGET_DIR := $(INITRAMFS)/opt/apparmor

.PHONY: all
all:
	@mkdir -p $(TARGET_DIR)
	@cp -r $(APPARMOR_PREBUILT_DIR)/. $(TARGET_DIR)/
