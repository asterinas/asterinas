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
# Specify whether to build regression tests under `test/src/apps`.
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

# NixOS settings
NIXOS_DISK_SIZE_IN_MB ?= 8192
NIXOS_DISABLE_SYSTEMD ?= false
NIXOS_TEST_COMMAND ?=
# The following option is only effective when NIXOS_DISABLE_SYSTEMD is set to 'true'.
# Use a login shell to ensure that environment variables are initialized correctly.
NIXOS_STAGE_2_INIT ?= /bin/sh -l
# End of NixOS settings

# ISO installer settings
AUTO_INSTALL ?= true
# End of ISO installer settings

# Cachix binary cache settings
CACHIX_AUTH_TOKEN ?=
RELEASE_CACHIX_NAME ?= "aster-nixos-release"
RELEASE_SUBSTITUTER ?= https://aster-nixos-release.cachix.org
RELEASE_TRUSTED_PUBLIC_KEY ?= aster-nixos-release.cachix.org-1:xB6U/f5ck5vGDJZ04kPp3zGpZ4Nro9X4+TSSMAETVFE=
DEV_CACHIX_NAME ?= "aster-nixos-dev"
DEV_SUBSTITUTER ?= https://aster-nixos-dev.cachix.org
DEV_TRUSTED_PUBLIC_KEY ?= aster-nixos-dev.cachix.org-1:xrCbE2flfliFTQCY/2HeJoT2tCO+5kMTZeLIUH9lnIA=
# End of Cachix binary cache settings

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
CARGO_OSDK_INITRAMFS_OPTION := --initramfs=$(abspath test/build/initramfs.cpio)
CARGO_OSDK_COMMON_ARGS += $(CARGO_OSDK_INITRAMFS_OPTION)
endif

CARGO_OSDK_BUILD_ARGS += $(CARGO_OSDK_COMMON_ARGS)
CARGO_OSDK_TEST_ARGS += $(CARGO_OSDK_COMMON_ARGS)

# Pass make variables to all subdirectory makes
export

# Basically, non-OSDK crates do not depend on Aster Frame and can be checked
# or tested without OSDK.
NON_OSDK_CRATES := \
	ostd/libs/align_ext \
	ostd/libs/id-alloc \
	ostd/libs/linux-bzimage/builder \
	ostd/libs/linux-bzimage/boot-params \
	ostd/libs/ostd-macros \
	ostd/libs/ostd-test \
	kernel/libs/aster-rights \
	kernel/libs/aster-rights-proc \
	kernel/libs/atomic-integer-wrapper \
	kernel/libs/cpio-decoder \
	kernel/libs/int-to-c-enum \
	kernel/libs/int-to-c-enum/derive \
	kernel/libs/jhash \
	kernel/libs/keyable-arc \
	kernel/libs/logo-ascii-art \
	kernel/libs/typeflags \
	kernel/libs/typeflags-util \
	tools/sctrace

# In contrast, OSDK crates depend on OSTD (or being `ostd` itself)
# and need to be built or tested with OSDK.
OSDK_CRATES := \
	osdk/deps/frame-allocator \
	osdk/deps/heap-allocator \
	osdk/deps/test-kernel \
	ostd \
	ostd/libs/linux-bzimage/setup \
	kernel \
	kernel/comps/block \
	kernel/comps/cmdline \
	kernel/comps/console \
	kernel/comps/framebuffer \
	kernel/comps/input \
	kernel/comps/i8042 \
	kernel/comps/network \
	kernel/comps/softirq \
	kernel/comps/systree \
	kernel/comps/logger \
	kernel/comps/mlsdisk \
	kernel/comps/time \
	kernel/comps/virtio \
	kernel/comps/pci \
	kernel/libs/aster-util \
	kernel/libs/aster-bigtcp \
	kernel/libs/device-id \
	kernel/libs/xarray

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
	@cd osdk && cargo clippy -- -D warnings

.PHONY: test_osdk
test_osdk:
	@cd osdk && \
		OSDK_LOCAL_DEV=1 cargo build && \
		OSDK_LOCAL_DEV=1 cargo test

.PHONY: check_vdso
check_vdso:
	@# Checking `VDSO_LIBRARY_DIR` environment variable
	@if [ -z "$(VDSO_LIBRARY_DIR)" ]; then \
		echo "Error: the VDSO_LIBRARY_DIR environment variable must be given."; \
		echo "    This variable points to a directory that provides Linux's vDSO files,"; \
		echo "    which is required to build Asterinas. Search for VDSO_LIBRARY_DIR"; \
		echo "    in Asterinas's Dockerfile for more information."; \
		exit 1; \
	fi

.PHONY: initramfs
initramfs: check_vdso
	@$(MAKE) --no-print-directory -C test

# Build the kernel with an initramfs
.PHONY: kernel
kernel: initramfs $(CARGO_OSDK)
	@cd kernel && cargo osdk build $(CARGO_OSDK_BUILD_ARGS)

# Build the kernel with an initramfs and then run it
.PHONY: run_kernel
run_kernel: initramfs $(CARGO_OSDK)
	@cd kernel && cargo osdk run $(CARGO_OSDK_BUILD_ARGS)
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

# Build the Asterinas NixOS ISO installer image
iso: BOOT_PROTOCOL = linux-efi-handover64
iso:
	@make kernel
	@./tools/nixos/build_iso.sh

# Build the Asterinas NixOS ISO installer image and then do installation
run_iso: OVMF = off
run_iso:
	@./tools/nixos/run_iso.sh

# Create an Asterinas NixOS installation on host
nixos: BOOT_PROTOCOL = linux-efi-handover64
nixos:
	@make kernel
	@./tools/nixos/build_nixos.sh

# After creating a Asterinas NixOS installation (via either the `run_iso` or `nixos` target),
# run the NixOS
run_nixos: OVMF = off
run_nixos:
	@./tools/nixos/run_nixos.sh target/nixos

# Build the Asterinas NixOS patched packages
cachix:
	@nix-build distro/cachix \
		--argstr test-command "${NIXOS_TEST_COMMAND}" \
		--option extra-substituters "${RELEASE_SUBSTITUTER} ${DEV_SUBSTITUTER}" \
		--option extra-trusted-public-keys "${RELEASE_TRUSTED_PUBLIC_KEY} ${DEV_TRUSTED_PUBLIC_KEY}" \
		--out-link cachix.list

# Push the Asterinas NixOS patched packages to Cachix
.PHONY: push_cachix
push_cachix: USE_RELEASE_CACHE ?= 0
push_cachix: cachix
ifeq ($(USE_RELEASE_CACHE), 1)
	@cachix push $(RELEASE_CACHIX_NAME) < cachix.list
else
	@cachix push $(DEV_CACHIX_NAME) < cachix.list
endif

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

.PHONY: test
test:
	@for dir in $(NON_OSDK_CRATES); do \
		(cd $$dir && cargo test) || exit 1; \
	done

.PHONY: ktest
ktest: initramfs $(CARGO_OSDK)
	@# Notes:
	@# 1. linux-bzimage-setup is excluded from ktest since it's hard to be unit tested;
	@# 2. Artifacts are removed after testing each crate to save the limited disk space
	@#    available to free-tier Github runners.
	@for dir in $(OSDK_CRATES); do \
		[ $$dir = "ostd/libs/linux-bzimage/setup" ] && continue; \
		echo "[make] Testing $$dir"; \
		(cd $$dir && cargo osdk test $(CARGO_OSDK_TEST_ARGS)) || exit 1; \
		tail --lines 10 qemu.log | grep -q "^\\[ktest runner\\] All crates tested." \
			|| (echo "Test failed" && exit 1); \
		rm -r target/osdk/*; \
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
		if [ "$$dir" = "kernel" ]; then \
			EXTRA_DOC_FLAGS="--document-private-items -Arustdoc::private_intra_doc_links"; \
		fi; \
		(cd $$dir && RUSTDOCFLAGS="-Dwarnings $$EXTRA_DOC_FLAGS" cargo osdk doc --no-deps) || exit 1; \
	done

.PHONY: book
book:
	@cd book && mdbook build

.PHONY: format
format:
	@./tools/format_all.sh
	@nixfmt ./distro
	@$(MAKE) --no-print-directory -C test format

.PHONY: check
check: initramfs $(CARGO_OSDK)
	@# Check formatting issues of the Rust code
	@./tools/format_all.sh --check
	@
	@# Check if the combination of STD_CRATES and NON_OSDK_CRATES is the
	@# same as all workspace members
	@sed -n '/^\[workspace\]/,/^\[.*\]/{/members = \[/,/\]/p}' Cargo.toml | \
		grep -v "members = \[" | tr -d '", \]' | \
		sort > /tmp/all_crates
	@echo $(NON_OSDK_CRATES) $(OSDK_CRATES) | tr ' ' '\n' | sort > /tmp/combined_crates
	@diff -B /tmp/all_crates /tmp/combined_crates || \
		(echo "Error: The combination of STD_CRATES and NOSTD_CRATES" \
			"is not the same as all workspace members" && exit 1)
	@rm /tmp/all_crates /tmp/combined_crates
	@
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
		(cd $$dir && cargo clippy --no-deps -- -D warnings) || exit 1; \
	done
	@for dir in $(OSDK_CRATES); do \
		echo "Checking $$dir"; \
		# Exclude linux-bzimage-setup since it only supports x86-64 currently and will panic \
		# in other architectures. \
		[ "$$dir" = "ostd/libs/linux-bzimage/setup" ] && [ "$(OSDK_TARGET_ARCH)" != "x86_64" ] && continue; \
		# Run clippy on each crate with and without the ktest configuration. \
		(cd $$dir && cargo osdk clippy -- --no-deps -- -D warnings) || exit 1; \
		(cd $$dir && cargo osdk clippy --ktest -- --no-deps -- -D warnings) || exit 1; \
	done
	@
	@# Check formatting issues of the C code and Nix files (regression tests)
	@$(MAKE) --no-print-directory -C test check
	@
	@# Check typos
	@typos
	@# Check formatting issues of Nix files under distro directory
	@nixfmt --check ./distro

.PHONY: clean
clean:
	@echo "Cleaning up Asterinas workspace target files"
	@cargo clean
	@echo "Cleaning up OSDK workspace target files"
	@cd osdk && cargo clean
	@echo "Cleaning up mdBook output files"
	@cd book && mdbook clean
	@echo "Cleaning up test target files"
	@$(MAKE) --no-print-directory -C test clean
	@echo "Uninstalling OSDK"
	@rm -f $(CARGO_OSDK)
