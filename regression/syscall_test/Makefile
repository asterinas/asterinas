TESTS ?= open_test read_test statfs_test chmod_test

MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
BUILD_DIR ?= $(CUR_DIR)/../build
ifdef JINUX_PREBUILT_SYSCALL_TEST
	BIN_DIR := $(JINUX_PREBUILT_SYSCALL_TEST)
else
	BIN_DIR := $(BUILD_DIR)/syscall_test_bins
	SRC_DIR := $(BUILD_DIR)/gvisor_src
endif
INITRAMFS ?= $(CUR_DIR)/../build/initramfs
TARGET_DIR := $(INITRAMFS)/opt/syscall_test
RUN_BASH := $(CUR_DIR)/run_syscall_test.sh
BLOCK_LIST := $(CUR_DIR)/blocklists

.PHONY: all clean

all: $(TESTS)

$(TESTS): $(BIN_DIR) $(TARGET_DIR)
	@cp -f $</$@ $(TARGET_DIR)/tests

ifndef JINUX_PREBUILT_SYSCALL_TEST
$(BIN_DIR): $(SRC_DIR)
	@if ! type bazel > /dev/null; then \
		echo "bazel is not installed, please run $(CUR_DIR)/install_bazel.sh with sudo permission to install it."; \
		exit 1; \
	fi
	@rm -rf $@ && mkdir -p $@
	@cd $(SRC_DIR) && bazel build --test_tag_filters=native //test/syscalls/...
	@cp $(SRC_DIR)/bazel-bin/test/syscalls/linux/*_test $@

$(SRC_DIR):
	@rm -rf $@ && mkdir -p $@
	@cd $@ && git clone -b 20200921.0 https://github.com/jinzhao-dev/gvisor.git .
endif

$(TARGET_DIR): $(RUN_BASH) $(BLOCK_LIST)
	@rm -rf $@ && mkdir -p $@
	@# Prepare tests dir for test binaries
	@mkdir $@/tests
	@# Copy blocklists
	@cp -rf $(BLOCK_LIST) $@
	@# Copy bash script
	@cp -f $(RUN_BASH) $@

clean:
	@rm -rf $(TARGET_DIR)