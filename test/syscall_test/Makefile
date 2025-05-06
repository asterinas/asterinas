# SPDX-License-Identifier: MPL-2.0

MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
INITRAMFS ?= $(CUR_DIR)/../build/initramfs
TARGET_DIR := $(INITRAMFS)/opt/syscall_test
RUN_BASH := $(CUR_DIR)/run_syscall_test.sh
WHITELIST := $(CUR_DIR)/whitelist.txt
ALL_SYSCALL_TESTS := /opt/ltp/runtest/syscalls

.PHONY: all
all: $(TARGET_DIR)

$(TARGET_DIR): $(RUN_BASH)
	@rm -rf $@ && mkdir -p $@
	@# Prepare tests dir for test binaries
	@mkdir -p $@/testcases/bin
	@mkdir -p $@/runtest
	@awk '!/^#/ && NF' $(WHITELIST) > $@/whitelist.txt
	@# Process syscalls to identify and copy binaries
	@echo "Processing syscalls and copying binaries..."
	@while read -r syscall binary params; do \
		if grep -q "^$$syscall$$" $@/whitelist.txt; then \
			if [ -f /opt/ltp/testcases/bin/$$binary ]; then \
				cp -f /opt/ltp/testcases/bin/$$binary $@/testcases/bin; \
				echo "$$syscall $$binary $$params" >> $@/runtest/syscalls; \
				echo "Copied testcase: $$syscall $$binary $$params"; \
			else \
				echo "Testcase not found: $$binary (skipping)"; \
			fi; \
		fi; \
	done < $(ALL_SYSCALL_TESTS)
	@cp -r /opt/ltp/bin $@
	@cp -f /opt/ltp/runltp $@
	@cp -f /opt/ltp/Version $@
	@cp -f /opt/ltp/ver_linux $@
	@cp -f /opt/ltp/IDcheck.sh $@
	@cp -f $(RUN_BASH) $@

.PHONY: clean
clean:
	@rm -rf $(TARGET_DIR)
