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

.PHONY: all
all: $(TARGET_DIR)

$(TARGET_DIR): $(RUN_BASH) $(ALL_TESTS) $(EXT2_BLOCKLIST) $(EXFAT_BLOCKLIST)
	@rm -rf $@ && mkdir -p $@
	@# Prepare tests dir for test binaries
	@mkdir -p $@/testcases/bin
	@mkdir -p $@/runtest
	@# When a selector is set, package only the selected tests and ignore the blocklist.
	@if [ -n "$(CONFORMANCE_TEST_SELECTOR)" ]; then \
		printf '%s\n' "$(CONFORMANCE_TEST_SELECTOR)" | tr ',' '\n' > $@/requested.txt; \
		selected_count=0; \
		invalid_selector=0; \
		while IFS= read -r syscall || [ -n "$$syscall" ]; do \
			syscall=$${syscall#"$${syscall%%[![:space:]]*}"}; \
			syscall=$${syscall%"$${syscall##*[![:space:]]}"}; \
			[ -z "$$syscall" ] && continue; \
			selected_count=$$((selected_count + 1)); \
			if awk -v selected="$$syscall" '$$1 == selected { found = 1 } END { exit !found }' $(LTP_PREBUILT_DIR)/runtest/syscalls; then \
				binary=$$(awk -v selected="$$syscall" '$$1 == selected { print $$2; exit }' $(LTP_PREBUILT_DIR)/runtest/syscalls); \
				if [ ! -f $(LTP_PREBUILT_DIR)/testcases/bin/$$binary ]; then \
					echo "Error: LTP test binary is not available: $$syscall ($$binary)" >&2; \
					invalid_selector=1; \
					continue; \
				fi; \
				printf '%s\n' "$$syscall" >> $@/filtered.txt; \
			else \
				echo "Error: unknown LTP test: $$syscall" >&2; \
				invalid_selector=1; \
			fi; \
		done < $@/requested.txt; \
		if [ "$$selected_count" -eq 0 ]; then \
			echo "Error: CONFORMANCE_TEST_SELECTOR contains no test names" >&2; \
			exit 2; \
		fi; \
		if [ "$$invalid_selector" -ne 0 ]; then \
			exit 2; \
		fi; \
	elif [ "$(CONFORMANCE_TEST_WORKDIR)" = "/ext2" ]; then \
		awk '!/^#/ && NF' $(ALL_TESTS) | grep -vxF -f $(EXT2_BLOCKLIST) > $@/filtered.txt; \
	elif [ "$(CONFORMANCE_TEST_WORKDIR)" = "/exfat" ]; then \
		awk '!/^#/ && NF' $(ALL_TESTS) | grep -vxF -f $(EXFAT_BLOCKLIST) > $@/filtered.txt; \
	else \
		awk '!/^#/ && NF' $(ALL_TESTS) > $@/filtered.txt; \
	fi;
	@# Process syscall testcases and copy binaries
	@while read -r syscall binary params; do \
		if grep -qxF "$$syscall" $@/filtered.txt; then \
			if [ -f $(LTP_PREBUILT_DIR)/testcases/bin/$$binary ]; then \
				cp -f $(LTP_PREBUILT_DIR)/testcases/bin/$$binary $@/testcases/bin; \
				echo "$$syscall $$binary $$params" >> $@/runtest/syscalls; \
			else \
				echo "Warning: $$binary not found (skipping)"; \
			fi; \
		fi; \
	done < $(LTP_PREBUILT_DIR)/runtest/syscalls
	@# Remove intermediate files
	@rm -f $@/filtered.txt $@/requested.txt
	@# Copy bash scripts
	@if [ -d $(LTP_PREBUILT_DIR)/bin ]; then \
		cp -r $(LTP_PREBUILT_DIR)/bin $@; \
	fi
	@cp -r $(LTP_PREBUILT_DIR)/libkirk $@
	@cp -f $(LTP_PREBUILT_DIR)/kirk $@
	@cp -f $(LTP_PREBUILT_DIR)/Version $@
	@cp -f $(LTP_PREBUILT_DIR)/ver_linux $@
	@cp -f $(LTP_PREBUILT_DIR)/IDcheck.sh $@
	@cp -f $(RUN_BASH) $@

.PHONY: clean
clean:
	@rm -rf $(TARGET_DIR)
