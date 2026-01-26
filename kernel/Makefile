# SPDX-License-Identifier: MPL-2.0

# This Makefile is responsible to build, run, test all kernel Rust crates.

SRC_ROOT := $(abspath ..)
SHELL := /bin/bash

# All crates under kernel/libs, kernel/comps, osdk/deps, ostd/libs
CRATE_DIRS := $(SRC_ROOT)/kernel/libs $(SRC_ROOT)/kernel/comps $(SRC_ROOT)/osdk/deps $(SRC_ROOT)/ostd/libs
# Find all crate directories and remove trailing slashes
ALL_CRATES := $(patsubst %/,%,$(sort $(foreach dir,$(CRATE_DIRS),\
	$(dir $(shell find $(dir) -name Cargo.toml -type f 2>/dev/null)))))
# Special OSDK crates do not directly rely on OSTD, but may depend on crates that do.
SPECIAL_OSDK_CRATES := \
	$(SRC_ROOT)/kernel/libs/device-id \
	$(SRC_ROOT)/ostd/libs/linux-bzimage/setup
# Exclude crates under kernel/libs/comp-sys/ and special OSDK crates
ALL_CRATES := $(filter-out \
	$(SRC_ROOT)/kernel/libs/comp-sys/% \
	$(SPECIAL_OSDK_CRATES), \
	$(ALL_CRATES))

# Basically, non-OSDK crates do not depend on OSTD and can be checked
# or tested without OSDK.
NON_OSDK_CRATES := $(strip \
	$(foreach crate,$(ALL_CRATES),\
		$(if $(shell grep -q 'ostd\.workspace\s*=\s*true' $(crate)/Cargo.toml 2>/dev/null && echo yes),,\
			$(crate))\
	)\
)

# In contrast, OSDK crates depend on OSTD (or being `ostd` itself)
# and need to be built or tested with OSDK.
OSDK_CRATES := $(SRC_ROOT)/kernel $(SRC_ROOT)/ostd $(SPECIAL_OSDK_CRATES) $(strip \
	$(foreach crate,$(ALL_CRATES),\
		$(if $(shell grep -q 'ostd\.workspace\s*=\s*true' $(crate)/Cargo.toml 2>/dev/null && echo yes),\
			$(crate),)\
	)\
)

.PHONY: all
all: build

.PHONY: build
build:
	@cargo osdk build $(CARGO_OSDK_BUILD_ARGS)

.PHONY: run
run:
	@cargo osdk run $(CARGO_OSDK_BUILD_ARGS)

.PHONY: check_crates_list
check_crates_list:
	@# Extract workspace members as relative paths (e.g., "kernel", "ostd/libs/foo")
	@sed -n '/^\[workspace\]/,/^\[.*\]/{/members = \[/,/\]/p}' $(SRC_ROOT)/Cargo.toml | \
		grep -v "members = \[" | tr -d '", \]' | \
		sort > /tmp/all_members

	@# Convert our absolute paths to relative paths (relative to SRC_ROOT)
	@echo $(NON_OSDK_CRATES) $(OSDK_CRATES) | tr ' ' '\n' | \
		sed 's|^$(SRC_ROOT)/||' | sort > /tmp/combined_members

	@# Compare
	@diff -B /tmp/all_members /tmp/combined_members || \
		(echo "Error: The combination of OSDK_CRATES and NON_OSDK_CRATES" \
			"is not the same as all workspace members" && exit 1)

	@rm -f /tmp/all_members /tmp/combined_members

.PHONY: check
check: check_crates_list
	@# Check if all workspace members enable workspace lints
	@for dir in $(NON_OSDK_CRATES) $(OSDK_CRATES); do \
		if [[ "$$(tail -2 $$dir/Cargo.toml)" != "[lints]"$$'\n'"workspace = true" ]]; then \
			echo "Error: Workspace lints in $$dir are not enabled"; \
			exit 1; \
		fi \
	done
	@
	@# Check compilation of the Rust code
	@for dir in $(NON_OSDK_CRATES); do \
		echo "Checking $$dir"; \
		# Run clippy on each crate with and without the test configuration. \
		(cd $$dir && cargo clippy --no-deps -- -D warnings) || exit 1; \
		(cd $$dir && cargo clippy --tests --no-deps -- -D warnings) || exit 1; \
	done
	@for dir in $(OSDK_CRATES); do \
		echo "Checking $$dir"; \
		# Exclude linux-bzimage-setup since it only supports x86-64 currently and will panic \
		# in other architectures. \
 		[ "$$dir" = "$(SRC_ROOT)/ostd/libs/linux-bzimage/setup" ] && [ "$(OSDK_TARGET_ARCH)" != "x86_64" ] && continue; \
 		# Run clippy on each crate with and without the ktest configuration. \
 		(cd $$dir && cargo osdk clippy -- --no-deps -- -D warnings) || exit 1; \
 		(cd $$dir && cargo osdk clippy --ktests -- --no-deps -- -D warnings) || exit 1; \
	done

.PHONY: test
test:
	@for dir in $(NON_OSDK_CRATES); do \
		(cd $$dir && cargo test) || exit 1; \
	done

.PHONY: ktest
ktest: 
	@# Notes:
	@# 1. linux-bzimage-setup is excluded from ktest since it's hard to be unit tested;
	@# 2. Artifacts are removed after testing each crate to save the limited disk space
	@#    available to free-tier Github runners.
	@for dir in $(OSDK_CRATES); do \
		[ $$dir = "$(SRC_ROOT)/ostd/libs/linux-bzimage/setup" ] && continue; \
		echo "[make] Testing $$dir"; \
		(cd $$dir && cargo osdk test $(CARGO_OSDK_TEST_ARGS)) || exit 1; \
		tail --lines 10 $(SRC_ROOT)/qemu.log | grep -q "^\\[ktest runner\\] All crates tested." \
			|| (echo "Test failed" && exit 1); \
		rm -rf target/osdk/*; \
	done

.PHONY: docs
docs: $(CARGO_OSDK)
	@for dir in $(NON_OSDK_CRATES); do \
		(cd $$dir && RUSTDOCFLAGS="-Dwarnings" cargo doc --no-deps) || exit 1; \
	done
	@for dir in $(OSDK_CRATES); do \
		EXTRA_DOC_FLAGS=""; \
		# The kernel crate is primarily composed of private items. \
		# We include the --document-private-items flag \
		# to ensure documentation of the internal items is fully checked. \
		if [ "$$dir" = "$(SRC_ROOT)/kernel" ]; then \
			EXTRA_DOC_FLAGS="--document-private-items -Arustdoc::private_intra_doc_links"; \
		fi; \
		(cd $$dir && RUSTDOCFLAGS="-Dwarnings $$EXTRA_DOC_FLAGS" cargo osdk doc --no-deps) || exit 1; \
	done
