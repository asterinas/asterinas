# SPDX-License-Identifier: MPL-2.0

# Global options.
ARCH ?= x86_64
BOOT_METHOD ?= grub-rescue-iso
BOOT_PROTOCOL ?= multiboot2
BUILD_SYSCALL_TEST ?= 0
ENABLE_KVM ?= 1
INTEL_TDX ?= 0
RELEASE_MODE ?= 0
SCHEME ?= ""
# End of global options.

# The Makefile provides a way to run arbitrary tests in the kernel
# mode using the kernel command line.
# Here are the options for the auto test feature.
AUTO_TEST ?= none
EXTRA_BLOCKLISTS_DIRS ?= ""
SYSCALL_TEST_DIR ?= /tmp
# End of auto test features.

CARGO_OSDK := ~/.cargo/bin/cargo-osdk

CARGO_OSDK_ARGS := --target-arch=$(ARCH)

ifeq ($(AUTO_TEST), syscall)
BUILD_SYSCALL_TEST := 1
CARGO_OSDK_ARGS += --kcmd-args="SYSCALL_TEST_DIR=$(SYSCALL_TEST_DIR)"
CARGO_OSDK_ARGS += --kcmd-args="EXTRA_BLOCKLISTS_DIRS=$(EXTRA_BLOCKLISTS_DIRS)"
CARGO_OSDK_ARGS += --init-args="/opt/syscall_test/run_syscall_test.sh"
else ifeq ($(AUTO_TEST), regression)
CARGO_OSDK_ARGS += --init-args="/regression/run_regression_test.sh"
else ifeq ($(AUTO_TEST), boot)
CARGO_OSDK_ARGS += --init-args="/regression/boot_hello.sh"
endif

ifeq ($(RELEASE_MODE), 1)
CARGO_OSDK_ARGS += --profile release
endif

ifeq ($(INTEL_TDX), 1)
CARGO_OSDK_ARGS += --features intel_tdx
endif

ifneq ($(SCHEME), "")
CARGO_OSDK_ARGS += --scheme $(SCHEME)
else
CARGO_OSDK_ARGS += --boot-method="$(BOOT_METHOD)"
endif

# To test the linux-efi-handover64 boot protocol, we need to use Debian's
# GRUB release, which is installed in /usr/bin in our Docker image.
ifeq ($(BOOT_PROTOCOL), linux-efi-handover64)
CARGO_OSDK_ARGS += --grub-mkrescue=/usr/bin/grub-mkrescue
CARGO_OSDK_ARGS += --grub-boot-protocol="linux"
else ifeq ($(BOOT_PROTOCOL), linux-legacy32)
CARGO_OSDK_ARGS += --linux-x86-legacy-boot
CARGO_OSDK_ARGS += --grub-boot-protocol="linux"
else
CARGO_OSDK_ARGS += --grub-boot-protocol=$(BOOT_PROTOCOL)
endif

ifeq ($(ENABLE_KVM), 1)
CARGO_OSDK_ARGS += --qemu-args="--enable-kvm"
endif

# Pass make variables to all subdirectory makes
export

# Basically, non-OSDK crates do not depend on Aster Frame and can be checked
# or tested without OSDK.
NON_OSDK_CRATES := \
	framework/libs/align_ext \
	framework/libs/aster-main \
	framework/libs/linux-bzimage/builder \
	framework/libs/linux-bzimage/boot-params \
	framework/libs/ktest \
	framework/libs/ktest-proc-macro \
	framework/libs/tdx-guest \
	kernel/libs/cpio-decoder \
	kernel/libs/int-to-c-enum \
	kernel/libs/int-to-c-enum/derive \
	kernel/libs/aster-rights \
	kernel/libs/aster-rights-proc \
	kernel/libs/keyable-arc \
	kernel/libs/typeflags \
	kernel/libs/typeflags-util

# In contrast, OSDK crates depend on Aster Frame (or being aster-frame itself)
# and need to be built or tested with OSDK.
OSDK_CRATES := \
	framework/aster-frame \
	framework/libs/linux-bzimage/setup \
	kernel \
	kernel/aster-nix \
	kernel/comps/block \
	kernel/comps/console \
	kernel/comps/framebuffer \
	kernel/comps/input \
	kernel/comps/network \
	kernel/comps/time \
	kernel/comps/virtio \
	kernel/libs/aster-util

.PHONY: all
all: build

# Install or update OSDK from source
# To uninstall, do `cargo uninstall cargo-osdk`
.PHONY: install_osdk
install_osdk:
	@cargo install cargo-osdk --path osdk

# This will install OSDK if it is not already installed
# To update OSDK, we need to run `install_osdk` manually
$(CARGO_OSDK):
	@make --no-print-directory install_osdk

.PHONY: initramfs
initramfs:
	@make --no-print-directory -C regression

.PHONY: build
build: initramfs $(CARGO_OSDK)
	@cargo osdk build $(CARGO_OSDK_ARGS)

.PHONY: tools
tools:
	@cd kernel/libs/comp-sys && cargo install --path cargo-component

.PHONY: run
run: build
	@cargo osdk run $(CARGO_OSDK_ARGS)
# Check the running status of auto tests from the QEMU log
ifeq ($(AUTO_TEST), syscall)
	@tail --lines 100 qemu.log | grep -q "^.* of .* test cases passed." || (echo "Syscall test failed" && exit 1)
else ifeq ($(AUTO_TEST), regression)
	@tail --lines 100 qemu.log | grep -q "^All regression tests passed." || (echo "Regression test failed" && exit 1)
else ifeq ($(AUTO_TEST), boot)
	@tail --lines 100 qemu.log | grep -q "^Successfully booted." || (echo "Boot test failed" && exit 1)
endif

.PHONY: gdb_server
gdb_server: build
	@cd kernel && cargo osdk run $(CARGO_OSDK_ARGS) -G --vsc --gdb-server-addr :1234

.PHONY: gdb_client
gdb_client: $(CARGO_OSDK)
	@cd kernel && cargo osdk debug $(CARGO_OSDK_ARGS) --remote :1234

.PHONY: test
test:
	@for dir in $(NON_OSDK_CRATES); do \
		(cd $$dir && cargo test) || exit 1; \
	done

.PHONY: ktest
ktest: initramfs $(CARGO_OSDK)
	@# Exclude linux-bzimage-setup from ktest since it's hard to be unit tested
	@for dir in $(OSDK_CRATES); do \
		[ $$dir = "framework/libs/linux-bzimage/setup" ] && continue; \
		(cd $$dir && cargo osdk test) || exit 1; \
	done

docs: $(CARGO_OSDK)
	@for dir in $(NON_OSDK_CRATES); do \
		(cd $$dir && cargo doc --no-deps) || exit 1; \
	done
	@for dir in $(OSDK_CRATES); do \
		(cd $$dir && cargo osdk doc --no-deps) || exit 1; \
	done
	@echo "" 								# Add a blank line
	@cd docs && mdbook build 				# Build mdBook

.PHONY: format
format:
	@./tools/format_all.sh
	@make --no-print-directory -C regression format

.PHONY: check
check: $(CARGO_OSDK)
	@cd osdk && cargo clippy -- -D warnings
	@./tools/format_all.sh --check   	# Check Rust format issues
	@# Check if STD_CRATES and NOSTD_CRATES combined is the same as all workspace members
	@sed -n '/^\[workspace\]/,/^\[.*\]/{/members = \[/,/\]/p}' Cargo.toml | \
		grep -v "members = \[" | tr -d '", \]' | \
		sort > /tmp/all_crates
	@echo $(NON_OSDK_CRATES) $(OSDK_CRATES) | tr ' ' '\n' | sort > /tmp/combined_crates
	@diff -B /tmp/all_crates /tmp/combined_crates || \
		(echo "Error: STD_CRATES and NOSTD_CRATES combined is not the same as all workspace members" && exit 1)
	@rm /tmp/all_crates /tmp/combined_crates
	@for dir in $(NON_OSDK_CRATES); do \
		echo "Checking $$dir"; \
		(cd $$dir && cargo clippy -- -D warnings) || exit 1; \
	done
	@for dir in $(OSDK_CRATES); do \
		echo "Checking $$dir"; \
		(cd $$dir && cargo osdk clippy -- -- -D warnings) || exit 1; \
	done
	@make --no-print-directory -C regression check

.PHONY: clean
clean:
	@cargo clean
	@cd docs && mdbook clean
	@make --no-print-directory -C regression clean
	@rm -f $(CARGO_OSDK)
