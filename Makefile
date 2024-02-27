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

# Maintain a list of usermode crates that can be tested with `cargo test`
USERMODE_TESTABLE := \
    framework/libs/align_ext \
    framework/libs/aster-main \
	framework/libs/linux-bzimage/builder \
	framework/libs/linux-bzimage/boot-params \
    framework/libs/ktest \
    framework/libs/ktest-proc-macro \
    services/libs/cpio-decoder \
    services/libs/int-to-c-enum \
    services/libs/int-to-c-enum/derive \
    services/libs/aster-rights \
    services/libs/aster-rights-proc \
    services/libs/keyable-arc \
    services/libs/typeflags \
    services/libs/typeflags-util

# Maintain a list of kernel crates that can be tested with `cargo osdk test`
# The framework is tested independently, thus not included here
KTEST_TESTABLE := \
    "services/aster-nix" \
    "services/comps/block" \
    "services/comps/console" \
    "services/comps/framebuffer" \
    "services/comps/input" \
    "services/comps/network" \
    "services/comps/time" \
    "services/comps/virtio"

.PHONY: all install_osdk build tools run test docs check clean update_initramfs

all: build

install_osdk:
	@cargo install cargo-osdk --path osdk

build:
	@make --no-print-directory -C regression
	@cargo osdk build $(CARGO_OSDK_ARGS)

tools:
	@cd services/libs/comp-sys && cargo install --path cargo-component

run: build
	@cargo osdk run $(CARGO_OSDK_ARGS)

test:
	@for dir in $(USERMODE_TESTABLE); do \
		(cd $$dir && cargo test) || exit 1; \
	done

ktest:
	@for dir in $(KTEST_TESTABLE); do \
		(cd $$dir && cargo osdk test) || exit 1; \
	done

docs:
	@cargo doc 								# Build Rust docs
	@echo "" 								# Add a blank line
	@cd docs && mdbook build 				# Build mdBook

format:
	./tools/format_all.sh

check:
	./tools/format_all.sh --check   # Check Rust format issues
	@cargo osdk clippy

clean:
	@cargo clean
	@cd docs && mdbook clean
	@make --no-print-directory -C regression clean

update_initramfs:
	@make --no-print-directory -C regression clean
	@make --no-print-directory -C regression
