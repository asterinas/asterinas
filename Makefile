# SPDX-License-Identifier: MPL-2.0

# The Makefile provides a way to run arbitrary tests in the kernel
# mode using the kernel command line.
# Here are the options for the auto test feature.
AUTO_TEST ?= none
BOOT_LOADER ?= grub
BOOT_PROTOCOL ?= multiboot2
QEMU_MACHINE ?= q35
BUILD_SYSCALL_TEST ?= 0
EMULATE_IOMMU ?= 0
ENABLE_KVM ?= 1
INTEL_TDX ?= 0
SKIP_GRUB_MENU ?= 1
SYSCALL_TEST_DIR ?= /tmp
RELEASE_MODE ?= 0
# End of auto test features.

CARGO_OSDK := ~/.cargo/bin/cargo-osdk

CARGO_OSDK_ARGS :=

ifeq ($(AUTO_TEST), syscall)
BUILD_SYSCALL_TEST := 1
CARGO_OSDK_ARGS += --kcmd_args="SYSCALL_TEST_DIR=$(SYSCALL_TEST_DIR)"
CARGO_OSDK_ARGS += --init_args="/opt/syscall_test/run_syscall_test.sh"
endif
ifeq ($(AUTO_TEST), regression)
CARGO_OSDK_ARGS += --init_args="/regression/run_regression_test.sh"
endif
ifeq ($(AUTO_TEST), boot)
CARGO_OSDK_ARGS += --init_args="-c exit 0"
endif

ifeq ($(RELEASE_MODE), 1)
CARGO_OSDK_ARGS += --profile release
endif

ifeq ($(INTEL_TDX), 1)
CARGO_OSDK_ARGS += --features intel_tdx
endif

CARGO_OSDK_ARGS += --boot.loader="$(BOOT_LOADER)"
CARGO_OSDK_ARGS += --boot.protocol="$(BOOT_PROTOCOL)"
CARGO_OSDK_ARGS += --qemu.machine="$(QEMU_MACHINE)"

ifeq ($(QEMU_MACHINE), microvm)
CARGO_OSDK_ARGS += --select microvm
endif

# To test the linux-efi-handover64 boot protocol, we need to use Debian's
# GRUB release, which is installed in /usr/bin in our Docker image.
ifeq ($(BOOT_PROTOCOL), linux-efi-handover64)
CARGO_OSDK_ARGS += --boot.grub-mkrescue=/usr/bin/grub-mkrescue
endif

ifeq ($(EMULATE_IOMMU), 1)
CARGO_OSDK_ARGS += --select iommu
endif

ifeq ($(ENABLE_KVM), 1)
CARGO_OSDK_ARGS += --qemu.args="--enable-kvm"
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

.PHONY: all build tools run test docs check clean update_initramfs install_osdk

all: build

# Install or update OSDK from source
# To uninstall, do `cargo uninstall cargo-osdk`
install_osdk:
	@cargo install cargo-osdk --path osdk

# This will install OSDK if it is not already installed
# To update OSDK, we need to run `install_osdk` manually
$(CARGO_OSDK):
	@make --no-print-directory install_osdk

build: $(CARGO_ODSK)
	@make --no-print-directory -C regression
	@cd kernel && cargo osdk build $(CARGO_OSDK_ARGS)

tools:
	@cd kernel/libs/comp-sys && cargo install --path cargo-component

run: build
	@cd kernel && cargo osdk run $(CARGO_OSDK_ARGS)

test:
	@for dir in $(NON_OSDK_CRATES); do \
		(cd $$dir && cargo test) || exit 1; \
	done

ktest: $(CARGO_ODSK)
	@# Exclude linux-bzimage-setup from ktest since it's hard to be unit tested
	@for dir in $(OSDK_CRATES); do \
		[ $$dir = "framework/libs/linux-bzimage/setup" ] && continue; \
		(cd $$dir && cargo osdk test) || exit 1; \
	done

docs:
	@cargo doc 								# Build Rust docs
	@echo "" 								# Add a blank line
	@cd docs && mdbook build 				# Build mdBook

format:
	@./tools/format_all.sh

check: $(CARGO_ODSK)
	@./tools/format_all.sh --check   	# Check Rust format issues
	@# Check if STD_CRATES and NOSTD_CRATES combined is the same as all workspace members
	@sed -n '/^\[workspace\]/,/^\[.*\]/{/members = \[/,/\]/p}' Cargo.toml | grep -v "members = \[" | tr -d '", \]' | sort > /tmp/all_crates
	@echo $(NON_OSDK_CRATES) $(OSDK_CRATES) | tr ' ' '\n' | sort > /tmp/combined_crates
	@diff -B /tmp/all_crates /tmp/combined_crates || (echo "Error: STD_CRATES and NOSTD_CRATES combined is not the same as all workspace members" && exit 1)
	@rm /tmp/all_crates /tmp/combined_crates
	@for dir in $(NON_OSDK_CRATES); do \
		(cd $$dir && cargo clippy -- -D warnings) || exit 1; \
	done
	@for dir in $(OSDK_CRATES); do \
		(cd $$dir && cargo osdk clippy) || exit 1; \
	done

clean:
	@cargo clean
	@cd docs && mdbook clean
	@make --no-print-directory -C regression clean

update_initramfs:
	@make --no-print-directory -C regression clean
	@make --no-print-directory -C regression
