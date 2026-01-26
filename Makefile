# SPDX-License-Identifier: MPL-2.0

# =========================== Makefile options. ===============================

# Global build options.
OSDK_TARGET_ARCH ?= x86_64
BENCHMARK ?= none
BOOT_METHOD ?= grub-rescue-iso
BOOT_PROTOCOL ?= multiboot2
BUILD_SYSCALL_TEST ?= 0
ENABLE_KVM ?= 1
INTEL_TDX ?= 0
MEM ?= 8G
OVMF ?= on
RELEASE ?= 0
RELEASE_LTO ?= 0
LOG_LEVEL ?= error
SCHEME ?= ""
SMP ?= 1
OSTD_TASK_STACK_SIZE_IN_PAGES ?= 64
FEATURES ?=
NO_DEFAULT_FEATURES ?= 0
COVERAGE ?= 0
# Specify whether to build regression tests under `test/initramfs/src/apps`.
ENABLE_BASIC_TEST ?= false
# Specify the primary system console (supported: hvc0, tty0).
# - hvc0: The virtio-console terminal.
# - tty0: The active virtual terminal (VT).
# Asterinas will automatically fall back to tty0 if hvc0 is not available.
# Note that currently the virtual terminal (tty0) can only work with
# linux-efi-handover64 and linux-efi-pe64 boot protocol.
CONSOLE ?= hvc0
# End of global build options.

# GDB debugging and profiling options.
GDB_TCP_PORT ?= 1234
GDB_PROFILE_FORMAT ?= flame-graph
GDB_PROFILE_COUNT ?= 200
GDB_PROFILE_INTERVAL ?= 0.1
# End of GDB options.

# The Makefile provides a way to run arbitrary tests in the kernel
# mode using the kernel command line.
# Here are the options for the auto test feature.
AUTO_TEST ?= none
EXTRA_BLOCKLISTS_DIRS ?= ""
SYSCALL_TEST_WORKDIR ?= /tmp
# End of auto test features.

# Network settings
# NETDEV possible values are user,tap
NETDEV ?= user
VHOST ?= off
# The name server listed by /etc/resolv.conf inside the Asterinas VM
DNS_SERVER ?= none
# End of network settings

# ========================= End of Makefile options. ==========================

SHELL := /bin/bash

CARGO_OSDK := ~/.cargo/bin/cargo-osdk

# Common arguments for `cargo osdk` `build`, `run` and `test` commands.
CARGO_OSDK_COMMON_ARGS := --target-arch=$(OSDK_TARGET_ARCH)
# The build arguments also apply to the `cargo osdk run` command.
CARGO_OSDK_BUILD_ARGS := --kcmd-args="ostd.log_level=$(LOG_LEVEL)"
CARGO_OSDK_BUILD_ARGS += --kcmd-args="console=$(CONSOLE)"
CARGO_OSDK_TEST_ARGS :=

ifeq ($(AUTO_TEST), syscall)
BUILD_SYSCALL_TEST := 1
CARGO_OSDK_BUILD_ARGS += --kcmd-args="SYSCALL_TEST_SUITE=$(SYSCALL_TEST_SUITE)"
CARGO_OSDK_BUILD_ARGS += --kcmd-args="SYSCALL_TEST_WORKDIR=$(SYSCALL_TEST_WORKDIR)"
CARGO_OSDK_BUILD_ARGS += --kcmd-args="EXTRA_BLOCKLISTS_DIRS=$(EXTRA_BLOCKLISTS_DIRS)"
CARGO_OSDK_BUILD_ARGS += --init-args="/opt/syscall_test/run_syscall_test.sh"
else ifeq ($(AUTO_TEST), test)
ENABLE_BASIC_TEST := true
	ifneq ($(SMP), 1)
	CARGO_OSDK_BUILD_ARGS += --kcmd-args="BLOCK_UNSUPPORTED_SMP_TESTS=1"
	endif
CARGO_OSDK_BUILD_ARGS += --kcmd-args="INTEL_TDX=$(INTEL_TDX)"
CARGO_OSDK_BUILD_ARGS += --init-args="/test/run_general_test.sh"
else ifeq ($(AUTO_TEST), boot)
ENABLE_BASIC_TEST := true
CARGO_OSDK_BUILD_ARGS += --init-args="/test/boot_hello.sh"
else ifeq ($(AUTO_TEST), vsock)
ENABLE_BASIC_TEST := true
export VSOCK=on
CARGO_OSDK_BUILD_ARGS += --init-args="/test/run_vsock_test.sh"
endif

ifeq ($(RELEASE_LTO), 1)
CARGO_OSDK_COMMON_ARGS += --profile release-lto
OSTD_TASK_STACK_SIZE_IN_PAGES = 8
else ifeq ($(RELEASE), 1)
CARGO_OSDK_COMMON_ARGS += --release
	ifeq ($(OSDK_TARGET_ARCH), riscv64)
	# FIXME: Unwinding in RISC-V seems to cost more stack space, so we increase
	# the stack size for it. This may need further investigation.
	# See https://github.com/asterinas/asterinas/pull/2383#discussion_r2307673156
	OSTD_TASK_STACK_SIZE_IN_PAGES = 16
	else
	OSTD_TASK_STACK_SIZE_IN_PAGES = 8
	endif
endif

# If the BENCHMARK is set, we will run the benchmark in the kernel mode.
ifneq ($(BENCHMARK), none)
CARGO_OSDK_BUILD_ARGS += --init-args="/benchmark/common/bench_runner.sh $(BENCHMARK) asterinas"
endif

ifeq ($(INTEL_TDX), 1)
BOOT_METHOD = grub-qcow2
BOOT_PROTOCOL = linux-efi-handover64
CARGO_OSDK_COMMON_ARGS += --scheme tdx
endif

ifeq ($(BOOT_PROTOCOL), linux-legacy32)
BOOT_METHOD = qemu-direct
OVMF = off
else ifeq ($(BOOT_PROTOCOL), multiboot)
BOOT_METHOD = qemu-direct
OVMF = off
endif

ifeq ($(SCHEME), "")
	ifeq ($(OSDK_TARGET_ARCH), riscv64)
	SCHEME = riscv
	else ifeq ($(OSDK_TARGET_ARCH), loongarch64)
	SCHEME = loongarch
	endif
endif

ifneq ($(SCHEME), "")
CARGO_OSDK_COMMON_ARGS += --scheme $(SCHEME)
else
CARGO_OSDK_COMMON_ARGS += --boot-method="$(BOOT_METHOD)"
endif

ifeq ($(COVERAGE), 1)
CARGO_OSDK_COMMON_ARGS += --coverage
endif

ifdef FEATURES
CARGO_OSDK_COMMON_ARGS += --features="$(FEATURES)"
endif
ifeq ($(NO_DEFAULT_FEATURES), 1)
CARGO_OSDK_COMMON_ARGS += --no-default-features
endif

# To test the linux-efi-handover64 boot protocol, we need to use Debian's
# GRUB release, which is installed in /usr/bin in our Docker image.
ifeq ($(BOOT_PROTOCOL), linux-efi-handover64)
CARGO_OSDK_COMMON_ARGS += --grub-mkrescue=/usr/bin/grub-mkrescue --grub-boot-protocol="linux"
else ifeq ($(BOOT_PROTOCOL), linux-efi-pe64)
CARGO_OSDK_COMMON_ARGS += --grub-boot-protocol="linux"
else ifeq ($(BOOT_PROTOCOL), linux-legacy32)
CARGO_OSDK_COMMON_ARGS += --linux-x86-legacy-boot --grub-boot-protocol="linux"
else
CARGO_OSDK_COMMON_ARGS += --grub-boot-protocol=$(BOOT_PROTOCOL)
endif

ifeq ($(ENABLE_KVM), 1)
	ifeq ($(OSDK_TARGET_ARCH), x86_64)
	CARGO_OSDK_COMMON_ARGS += --qemu-args="-accel kvm"
	endif
endif

# Skip GZIP to make encoding and decoding of initramfs faster
ifeq ($(INITRAMFS_SKIP_GZIP),1)
CARGO_OSDK_INITRAMFS_OPTION := --initramfs=$(abspath test/initramfs/build/initramfs.cpio)
CARGO_OSDK_COMMON_ARGS += $(CARGO_OSDK_INITRAMFS_OPTION)
endif

CARGO_OSDK_BUILD_ARGS += $(CARGO_OSDK_COMMON_ARGS)
CARGO_OSDK_TEST_ARGS += $(CARGO_OSDK_COMMON_ARGS)

# Pass make variables to all subdirectory makes
export

# OSDK dependencies
OSDK_SRC_FILES := \
	$(shell find osdk/Cargo.toml osdk/Cargo.lock osdk/src -type f)

.PHONY: all
all: kernel

# Install or update OSDK from source
# To uninstall, do `cargo uninstall cargo-osdk`
.PHONY: install_osdk
install_osdk:
	@# The `OSDK_LOCAL_DEV` environment variable is used for local development
	@# without the need to publish the changes of OSDK's self-hosted
	@# dependencies to `crates.io`.
	@OSDK_LOCAL_DEV=1 cargo install cargo-osdk --path osdk

# This will install and update OSDK automatically
$(CARGO_OSDK): $(OSDK_SRC_FILES)
	@$(MAKE) --no-print-directory install_osdk

.PHONY: check_osdk
check_osdk:
	@# Run clippy on OSDK with and without the test configuration.
	@cd osdk && cargo clippy --no-deps -- -D warnings
	@cd osdk && cargo clippy --tests --no-deps -- -D warnings

.PHONY: test_osdk
test_osdk:
	@cd osdk && \
		OSDK_LOCAL_DEV=1 cargo build && \
		OSDK_LOCAL_DEV=1 cargo test

.PHONY: initramfs
initramfs:
	@$(MAKE) --no-print-directory -C test/initramfs

# =========================== Kernel targets ===============================

# Build the kernel with an initramfs
.PHONY: kernel
kernel: initramfs $(CARGO_OSDK)
	@$(MAKE) --no-print-directory -C kernel

# Build the kernel with an initramfs and then run it
.PHONY: run_kernel
run_kernel: initramfs $(CARGO_OSDK)
	@$(MAKE) --no-print-directory -C kernel run
# Check the running status of auto tests from the QEMU log
ifeq ($(AUTO_TEST), syscall)
	@tail --lines 100 qemu.log | grep -q "^All syscall tests passed." \
		|| (echo "Syscall test failed" && exit 1)
else ifeq ($(AUTO_TEST), test)
	@tail --lines 100 qemu.log | grep -q "^All general tests passed." \
		|| (echo "General test failed" && exit 1)
else ifeq ($(AUTO_TEST), boot)
	@tail --lines 100 qemu.log | grep -q "^Successfully booted." \
		|| (echo "Boot test failed" && exit 1)
else ifeq ($(AUTO_TEST), vsock)
	@tail --lines 100 qemu.log | grep -q "^Vsock test passed." \
		|| (echo "Vsock test failed" && exit 1)
endif

# Run user-space unit tests for Rust crates not depending on OSTD
.PHONY: test
test:
	@$(MAKE) --no-print-directory -C kernel test

# Run kernel-space unix tests for Rust crates depending on OSTD
.PHONY: ktest
ktest: initramfs $(CARGO_OSDK)
	@$(MAKE) --no-print-directory -C kernel ktest

# Generate and check documentation for all crates
.PHONY: docs
docs: $(CARGO_OSDK)
	@$(MAKE) --no-print-directory -C kernel docs

# =========================== End of Kernel targets ===============================

# ============================== Distro targets ==================================

# Build the Asterinas NixOS ISO installer image
iso: BOOT_PROTOCOL := linux-efi-handover64
iso: 
	@$(MAKE) kernel
	@$(MAKE) --no-print-directory -C distro iso

# Build the Asterinas NixOS ISO installer image and then do installation
run_iso:
	@$(MAKE) --no-print-directory -C distro run_iso

# Create an Asterinas NixOS installation on host
nixos: BOOT_PROTOCOL := linux-efi-handover64
nixos:
	@$(MAKE) kernel
	@$(MAKE) --no-print-directory -C distro nixos

# After creating a Asterinas NixOS installation (via either the `run_iso` or `nixos` target),
# run the NixOS
run_nixos:
	@$(MAKE) --no-print-directory -C distro run_nixos

# Build the Asterinas NixOS patched packages
cachix:
	@$(MAKE) --no-print-directory -C distro cachix

# Push the Asterinas NixOS patched packages to Cachix
push_cachix: cachix
	@$(MAKE) --no-print-directory -C distro push_cachix

# =========================== End of Distro targets ===============================

.PHONY: gdb_server
gdb_server: initramfs $(CARGO_OSDK)
	@cd kernel && cargo osdk run $(CARGO_OSDK_BUILD_ARGS) --gdb-server wait-client,vscode,addr=:$(GDB_TCP_PORT)

.PHONY: gdb_client
gdb_client: initramfs $(CARGO_OSDK)
	@cd kernel && cargo osdk debug $(CARGO_OSDK_BUILD_ARGS) --remote :$(GDB_TCP_PORT)

.PHONY: profile_server
profile_server: initramfs $(CARGO_OSDK)
	@cd kernel && cargo osdk run $(CARGO_OSDK_BUILD_ARGS) --gdb-server addr=:$(GDB_TCP_PORT)

.PHONY: profile_client
profile_client: initramfs $(CARGO_OSDK)
	@cd kernel && cargo osdk profile $(CARGO_OSDK_BUILD_ARGS) --remote :$(GDB_TCP_PORT) \
		--samples $(GDB_PROFILE_COUNT) --interval $(GDB_PROFILE_INTERVAL) --format $(GDB_PROFILE_FORMAT)

.PHONY: book
book:
	@cd book && mdbook build

.PHONY: format
format:
	@./tools/format_rust.sh
	@$(MAKE) --no-print-directory -C distro format
	@$(MAKE) --no-print-directory -C test/initramfs format
	@$(MAKE) --no-print-directory -C test/nixos format

.PHONY: check
check: initramfs $(CARGO_OSDK)
	@# Check formatting issues of the Rust code
	@./tools/format_rust.sh --check
	@
	@# Check compilation of the Rust code
	@$(MAKE) --no-print-directory -C kernel check
	@
	@# Check formatting issues of Nix files under distro directory
	@$(MAKE) --no-print-directory -C distro check
	@
	@# Check formatting issues of the C code and Nix files (regression tests)
	@$(MAKE) --no-print-directory -C test/initramfs check
	@
	@# Check formatting issues of the Rust code in NixOS tests
	@$(MAKE) --no-print-directory -C test/nixos check
	@
	@# Check typos
	@typos

.PHONY: clean
clean:
	@echo "Cleaning up distro built files"
	@$(MAKE) --no-print-directory -C distro clean
	@echo "Cleaning up Asterinas workspace target files"
	@cargo clean
	@echo "Cleaning up OSDK workspace target files"
	@cd osdk && cargo clean
	@echo "Cleaning up mdBook output files"
	@cd book && mdbook clean
	@echo "Cleaning up test target files"
	@$(MAKE) --no-print-directory -C test/initramfs clean
	@echo "Uninstalling OSDK"
	@rm -f $(CARGO_OSDK)
