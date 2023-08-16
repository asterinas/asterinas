# Make arguments and their defaults
AUTO_SYSCALL_TEST ?= 0
BUILD_SYSCALL_TEST ?= 0
ENABLE_COVERAGE ?= 0
EMULATE_IOMMU ?= 0
ENABLE_KVM ?= 1
OPT_LEVEL ?= 0
# End of Make arguments

KERNEL_CMDLINE := SHELL="/bin/sh" LOGNAME="root" HOME="/" USER="root" PATH="/bin" init=/usr/bin/busybox -- sh -l
ifeq ($(AUTO_SYSCALL_TEST), 1)
KERNEL_CMDLINE += /opt/syscall_test/run_syscall_test.sh
endif

RUSTFLAGS := -Copt-level=$(OPT_LEVEL)
CARGO_KBUILD_ARGS :=
CARGO_KRUN_ARGS :=
JINUX_RUNNER_ARGS := '$(KERNEL_CMDLINE)'

ifeq ($(ENABLE_COVERAGE), 1)
RUSTFLAGS += -Cinstrument-coverage -Zno-profiler-runtime
CARGO_KBUILD_ARGS += --features jinux-std/coverage
CARGO_KRUN_ARGS += --features jinux-std/coverage
endif

ifeq ($(ENABLE_KVM), 1)
JINUX_RUNNER_ARGS += --enable-kvm
endif

ifeq ($(EMULATE_IOMMU), 1)
JINUX_RUNNER_ARGS += --emulate-iommu
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
	@RUSTFLAGS="$(RUSTFLAGS)" cargo kbuild $(CARGO_KBUILD_ARGS)

tools:
	@cd services/libs/comp-sys && cargo install --path cargo-component

run: build
	@RUSTFLAGS="$(RUSTFLAGS)" cargo krun  $(CARGO_KRUN_ARGS) -- $(JINUX_RUNNER_ARGS)

test: build
	@RUSTFLAGS="$(RUSTFLAGS)" cargo ktest $(CARGO_KRUN_ARGS) -- $(JINUX_RUNNER_ARGS) --do-kmode-test

docs:
	@cargo doc 								# Build Rust docs
	@echo "" 								# Add a blank line
	@cd docs && mdbook build 				# Build mdBook

check:
	@cargo fmt --check 				# Check Rust format issues
	@cargo kclippy					# Check common programming mistakes

clean:
	@cargo clean
	@cd docs && mdbook clean
	@make --no-print-directory -C regression clean
