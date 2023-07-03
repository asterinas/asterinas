TESTS ?= open_test read_test statfs_test chmod_test

MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
BUILD_DIR := $(CUR_DIR)/../build
SRC_DIR := $(BUILD_DIR)/gvisor_src
BIN_DIR := $(BUILD_DIR)/syscall_test_bins
INITRAMFS ?= $(CUR_DIR)/../build/initramfs
TARGET_DIR := $(INITRAMFS)/opt/syscall_test
RUN_BASH := $(CUR_DIR)/run_syscall_test.sh
BLOCK_LIST := $(CUR_DIR)/blocklists

.PHONY: all clean

all: $(TESTS)

$(SRC_DIR):
	@if ! type bazel > /dev/null; then \
		echo "bazel is not installed, please run $(CUR_DIR)/install_bazel.sh with sudo permission to install it."; \
		exit 1 ; \
	fi
	@rm -rf $@ && mkdir -p $@
	@cd $@ && git clone -b 20200921.0 https://github.com/jinzhao-dev/gvisor.git .

$(BIN_DIR): $(SRC_DIR)
	@rm -rf $@ && mkdir -p $@
	@cd $(SRC_DIR) && bazel build --test_tag_filters=native //test/syscalls/...
	@cp $(SRC_DIR)/bazel-bin/test/syscalls/linux/*_test $@

$(TARGET_DIR): $(RUN_BASH) $(BLOCK_LIST) $(BIN_DIR)
	@rm -rf $@ && mkdir -p $@
	@# Prepare tests dir for test binaries
	@mkdir $@/tests
	@# Copy blocklists
	@cp -rf $(BLOCK_LIST) $@
	@# Copy bash script
	@cp -f $(RUN_BASH) $@

$(TESTS): $(TARGET_DIR)
	@cp -f $(BIN_DIR)/$@ $(TARGET_DIR)/tests

clean:
	@rm -rf $(BIN_DIR) $(TARGET_DIR)
