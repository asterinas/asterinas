# SPDX-License-Identifier: MPL-2.0

MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
INITRAMFS ?= $(CUR_DIR)/../../build/initramfs
TARGET_DIR := $(INITRAMFS)/opt/ltp
RUN_BASH := $(CUR_DIR)/run_ltp_test.sh
WHITELIST := $(CUR_DIR)/whitelist.txt
EXT2_BLOCKLIST := $(CUR_DIR)/blocklist.ext2
EXFAT_BLOCKLIST := $(CUR_DIR)/blocklist.exfat
ALL_SYSCALL_TESTS := /opt/ltp/runtest/syscalls

.PHONY: all
all: $(TARGET_DIR)

$(TARGET_DIR): $(RUN_BASH) $(WHITELIST) $(EXT2_BLOCKLIST) $(EXFAT_BLOCKLIST)
	@rm -rf $@ && mkdir -p $@
	@# Prepare tests dir for test binaries
	@mkdir -p $@/testcases/bin
	@mkdir -p $@/runtest
	@awk '!/^#/ && NF' $(WHITELIST) > $@/whitelist.txt
	@if [ "$(SYSCALL_TEST_DIR)" = "/ext2" ]; then \
		grep -vxF -f $(EXT2_BLOCKLIST) $@/whitelist.txt > $@/filtered.txt; \
	elif [ "$(SYSCALL_TEST_DIR)" = "/exfat" ]; then \
		grep -vxF -f $(EXFAT_BLOCKLIST) $@/whitelist.txt > $@/filtered.txt; \
	else \
		cp -f $@/whitelist.txt $@/filtered.txt; \
	fi;
	@# Process syscalls to identify and copy binaries
	@while read -r syscall binary params; do \
		if grep -q "^$$syscall$$" $@/filtered.txt; then \
			if [ -f /opt/ltp/testcases/bin/$$binary ]; then \
				cp -f /opt/ltp/testcases/bin/$$binary $@/testcases/bin; \
				echo "$$syscall $$binary $$params" >> $@/runtest/syscalls; \
			else \
				echo "Warning: $$binary not found (skipping)"; \
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
