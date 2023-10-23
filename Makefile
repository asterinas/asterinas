# Make arguments and their defaults
AUTO_TEST ?= 0
BOOT_METHOD ?= qemu-grub
BOOT_PROTOCOL ?= multiboot2
BUILD_SYSCALL_TEST ?= 0
EMULATE_IOMMU ?= 0
ENABLE_KVM ?= 1
GDB_CLIENT ?= 0
GDB_SERVER ?= 0
INTEL_TDX ?= 0
SKIP_GRUB_MENU ?= 1
# End of Make arguments

KERNEL_CMDLINE := SHELL="/bin/sh" LOGNAME="root" HOME="/" USER="root" PATH="/bin" init=/usr/bin/busybox -- sh -l
ifeq ($(AUTO_TEST), syscall)
BUILD_SYSCALL_TEST := 1
KERNEL_CMDLINE += /opt/syscall_test/run_syscall_test.sh
endif
ifeq ($(AUTO_TEST), dummy)
KERNEL_CMDLINE += -c exit 0
endif

CARGO_KBUILD_ARGS :=

CARGO_KRUN_ARGS := -- '$(KERNEL_CMDLINE)'

CARGO_KRUN_ARGS += --boot-method="$(BOOT_METHOD)"
CARGO_KRUN_ARGS += --boot-protocol="$(BOOT_PROTOCOL)"

ifeq ($(EMULATE_IOMMU), 1)
CARGO_KRUN_ARGS += --emulate-iommu
endif

ifeq ($(ENABLE_KVM), 1)
CARGO_KRUN_ARGS += --enable-kvm
endif

ifeq ($(GDB_SERVER), 1)
ENABLE_KVM := 0
CARGO_KRUN_ARGS += --halt-for-gdb
endif

ifeq ($(GDB_CLIENT), 1)
CARGO_KRUN_ARGS += --run-gdb-client
endif

ifeq ($(INTEL_TDX), 1)
CARGO_KBUILD_ARGS += --features intel_tdx
CARGO_KRUN_ARGS += --features intel_tdx
endif

ifeq ($(SKIP_GRUB_MENU), 1)
CARGO_KRUN_ARGS += --skip-grub-menu
endif

# Pass make variables to all subdirectory makes
export

# Toolchain variables that are used when building the Linux setup header
export CARGO := cargo

.PHONY: all setup build tools run test docs check clean

all: build

setup:
	@rustup component add rust-src
	@rustup component add rustc-dev
	@rustup component add llvm-tools-preview
	@cargo install mdbook

build:
	@make --no-print-directory -C regression
	@cargo kbuild $(CARGO_KBUILD_ARGS)

tools:
	@cd services/libs/comp-sys && cargo install --path cargo-component

run: build
	@cargo krun $(CARGO_KRUN_ARGS)

test: build
	@cargo ktest

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
