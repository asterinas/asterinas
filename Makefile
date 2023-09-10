# Make arguments and their defaults
AUTO_SYSCALL_TEST ?= 0
BUILD_SYSCALL_TEST ?= 0
EMULATE_IOMMU ?= 0
ENABLE_KVM ?= 1
INTEL_TDX ?= 0
# End of Make arguments

KERNEL_CMDLINE := SHELL="/bin/sh" LOGNAME="root" HOME="/" USER="root" PATH="/bin" init=/usr/bin/busybox -- sh -l
ifeq ($(AUTO_SYSCALL_TEST), 1)
KERNEL_CMDLINE += /opt/syscall_test/run_syscall_test.sh
endif

CARGO_KBUILD_ARGS :=

CARGO_KRUN_ARGS := -- '$(KERNEL_CMDLINE)'

ifeq ($(INTEL_TDX), 1)
CARGO_KBUILD_ARGS += --features intel_tdx
CARGO_KRUN_ARGS += --features intel_tdx
endif

ifeq ($(ENABLE_KVM), 1)
CARGO_KRUN_ARGS += --enable-kvm
endif

ifeq ($(EMULATE_IOMMU), 1)
CARGO_KRUN_ARGS += --emulate-iommu
endif

ifeq ($(AUTO_SYSCALL_TEST), 1)
BUILD_SYSCALL_TEST := 1
endif

# Pass make variables to all subdirectory makes
export

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
