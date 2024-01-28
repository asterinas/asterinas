# SPDX-License-Identifier: MPL-2.0

# Make varaiables and defaults, you should refer to aster-runner for more details
AUTO_TEST ?= none
BOOT_METHOD ?= qemu-grub
BOOT_PROTOCOL ?= multiboot2
BUILD_SYSCALL_TEST ?= 0
EMULATE_IOMMU ?= 0
ENABLE_KVM ?= 1
GDB_CLIENT ?= 0
GDB_SERVER ?= 0
INTEL_TDX ?= 0
KTEST ?= 0
KTEST_CRATES ?= all
KTEST_WHITELIST ?=
SKIP_GRUB_MENU ?= 1
SYSCALL_TEST_DIR ?= /tmp
RELEASE_MODE ?= 0
# End of setting up Make varaiables

KERNEL_CMDLINE := SHELL="/bin/sh" LOGNAME="root" HOME="/" USER="root" PATH="/bin:/benchmark" init=/usr/bin/busybox
KERNEL_CMDLINE += ktest.whitelist="$(KTEST_WHITELIST)"
INIT_CMDLINE := sh -l
ifeq ($(AUTO_TEST), syscall)
BUILD_SYSCALL_TEST := 1
KERNEL_CMDLINE += SYSCALL_TEST_DIR=$(SYSCALL_TEST_DIR)
INIT_CMDLINE += /opt/syscall_test/run_syscall_test.sh
endif
ifeq ($(AUTO_TEST), regression)
INIT_CMDLINE += /regression/run_regression_test.sh
endif
ifeq ($(AUTO_TEST), boot)
INIT_CMDLINE += -c exit 0
endif

CARGO_KBUILD_ARGS :=
CARGO_KRUN_ARGS :=
GLOBAL_RUSTC_FLAGS :=

ifeq ($(RELEASE_MODE), 1)
CARGO_KBUILD_ARGS += --release
CARGO_KRUN_ARGS += --release
endif

ifeq ($(INTEL_TDX), 1)
CARGO_KBUILD_ARGS += --features intel_tdx
CARGO_KRUN_ARGS += --features intel_tdx
endif

RUNNER_ARGS := '$(KERNEL_CMDLINE) -- $(INIT_CMDLINE)'
RUNNER_ARGS += --boot-method="$(BOOT_METHOD)"
RUNNER_ARGS += --boot-protocol="$(BOOT_PROTOCOL)"

ifeq ($(EMULATE_IOMMU), 1)
RUNNER_ARGS += --emulate-iommu
endif

ifeq ($(ENABLE_KVM), 1)
RUNNER_ARGS += --enable-kvm
endif

ifeq ($(GDB_SERVER), 1)
ENABLE_KVM := 0
RUNNER_ARGS += --halt-for-gdb
endif

ifeq ($(GDB_CLIENT), 1)
RUNNER_ARGS += --run-gdb-client
endif

ifeq ($(KTEST), 1)
comma := ,
GLOBAL_RUSTC_FLAGS += --cfg ktest --cfg ktest=\"$(subst $(comma),\" --cfg ktest=\",$(KTEST_CRATES))\"
endif

ifeq ($(SKIP_GRUB_MENU), 1)
RUNNER_ARGS += --skip-grub-menu
endif

# Pass make variables to all subdirectory makes
export

# Toolchain variables that are used when building the Linux setup header
export CARGO := cargo

# Maintain a list of usermode crates that can be tested with `cargo test`
USERMODE_TESTABLE := \
    runner \
    framework/libs/align_ext \
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

.PHONY: all setup build tools run test docs check clean update_initramfs

all: build

setup:
	@rustup component add rust-src
	@rustup component add rustc-dev
	@rustup component add llvm-tools-preview
	@cargo install mdbook

build:
	@make --no-print-directory -C regression
	@RUSTFLAGS="$(GLOBAL_RUSTC_FLAGS)" cargo kbuild $(CARGO_KBUILD_ARGS)

tools:
	@cd services/libs/comp-sys && cargo install --path cargo-component

run: build
	@RUSTFLAGS="$(GLOBAL_RUSTC_FLAGS)" cargo krun $(CARGO_KRUN_ARGS) -- $(RUNNER_ARGS)

test:
	@for dir in $(USERMODE_TESTABLE); do \
		(cd $$dir && cargo test) || exit 1; \
	done

docs:
	@cargo doc 								# Build Rust docs
	@echo "" 								# Add a blank line
	@cd docs && mdbook build 				# Build mdBook

check:
	@cargo fmt --check              # Check Rust format issues
	@cargo kclippy -- -D warnings   # Make build fail if any warnings are found by rustc and clippy

clean:
	@cargo clean
	@cd docs && mdbook clean
	@make --no-print-directory -C regression clean

update_initramfs:
	@make --no-print-directory -C regression clean
	@make --no-print-directory -C regression