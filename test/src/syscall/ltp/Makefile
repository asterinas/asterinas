# SPDX-License-Identifier: MPL-2.0

LTP_PREBUILT_DIR ?= /opt/ltp
MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
INITRAMFS ?= $(CUR_DIR)/../../../build/initramfs
TARGET_DIR := $(INITRAMFS)/opt/ltp
RUN_BASH := $(CUR_DIR)/run_ltp_test.sh
ALL_TESTS := $(CUR_DIR)/testcases/all.txt
EXT2_BLOCKLIST := $(CUR_DIR)/testcases/blocked/ext2.txt
EXFAT_BLOCKLIST := $(CUR_DIR)/testcases/blocked/exfat.txt
SMP_BLOCKLIST := $(CUR_DIR)/testcases/blocked/smp.txt

.PHONY: all
all: $(TARGET_DIR)

$(TARGET_DIR): $(RUN_BASH) $(ALL_TESTS) $(EXT2_BLOCKLIST) $(EXFAT_BLOCKLIST) $(SMP_BLOCKLIST)
	@rm -rf $@ && mkdir -p $@
	@# Prepare tests dir for test binaries
	@mkdir -p $@/testcases/bin
	@mkdir -p $@/runtest
	@awk '!/^#/ && NF' $(ALL_TESTS) > $@/all.txt
	@if [ "$(SYSCALL_TEST_WORKDIR)" = "/ext2" ]; then \
		grep -vxF -f $(EXT2_BLOCKLIST) $@/all.txt > $@/filtered.txt; \
	elif [ "$(SYSCALL_TEST_WORKDIR)" = "/exfat" ]; then \
		grep -vxF -f $(EXFAT_BLOCKLIST) $@/all.txt > $@/filtered.txt; \
	else \
		cp -f $@/all.txt $@/filtered.txt; \
	fi;
	@if [ "$(SMP)" != "1" ]; then \
		grep -vxF -f $(SMP_BLOCKLIST) $@/filtered.txt > $@/filtered.txt.tmp; \
		mv $@/filtered.txt.tmp $@/filtered.txt; \
	fi;
	@# Process syscall testcases and copy binaries
	@while read -r syscall binary params; do \
		if grep -q "^$$syscall$$" $@/filtered.txt; then \
			if [ -f $(LTP_PREBUILT_DIR)/testcases/bin/$$binary ]; then \
				cp -f $(LTP_PREBUILT_DIR)/testcases/bin/$$binary $@/testcases/bin; \
				echo "$$syscall $$binary $$params" >> $@/runtest/syscalls; \
			else \
				echo "Warning: $$binary not found (skipping)"; \
			fi; \
		fi; \
	done < $(LTP_PREBUILT_DIR)/runtest/syscalls
	@# Remove intermediate files
	@rm -f $@/all.txt $@/filtered.txt
	@# Copy bash scripts
	@cp -r $(LTP_PREBUILT_DIR)/bin $@
	@cp -f $(LTP_PREBUILT_DIR)/runltp $@
	@cp -f $(LTP_PREBUILT_DIR)/Version $@
	@cp -f $(LTP_PREBUILT_DIR)/ver_linux $@
	@cp -f $(LTP_PREBUILT_DIR)/IDcheck.sh $@
	@cp -f $(RUN_BASH) $@

.PHONY: clean
clean:
	@rm -rf $(TARGET_DIR)
