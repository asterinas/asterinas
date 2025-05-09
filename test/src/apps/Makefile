# SPDX-License-Identifier: MPL-2.0

HOST_PLATFORM ?= x86_64-linux

# Prevent the implicit rules from compiling ".c" or ".s" files automatically.
MAKEFLAGS += --no-builtin-rules

MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))

INITRAMFS ?= $(CUR_DIR)/../../build/initramfs
TEST_BUILD_DIR ?= $(INITRAMFS)/test

# These test apps are sorted by name
TEST_APPS := \
	alarm \
	capability \
	clone3 \
	cpu_affinity \
	epoll \
	eventfd2 \
	execve \
	exit \
	fdatasync \
	file_io \
	fork_c \
	fb \
	getcpu \
	getpid \
	hello_pie \
	itimer \
	mmap \
	mongoose \
	network \
	pipe \
	prctl \
	process \
	pthread \
	pty \
	sched \
	shm \
	vsock \

# TODO: Refactor those tests for target CPU arch using C macro-based conditional compilation.
ifeq ($(HOST_PLATFORM), x86_64-linux)
TEST_APPS += \
	fork \
	hello_c \
	hello_world \
	signal_c
endif

# The C head and source files of all the apps, excluding the downloaded mongoose files
C_SOURCES := \
	$(shell find . -type f \( -name "*.c" -or -name "*.h" \) \
		! -name "mongoose.c" ! -name "mongoose.h")

.PHONY: all
all: $(TEST_APPS) scripts

.PHONY: $(TEST_APPS)
$(TEST_APPS):
	@make --no-print-directory -C $@

.PHONY: format
format:
	@echo "Fixing code format for general tests..."
	@clang-format -i $(C_SOURCES)

.PHONY: check
check:
	@echo "Checking code format for general tests..."
	@clang-format --dry-run --Werror $(C_SOURCES)

$(TEST_BUILD_DIR):
	@mkdir -p $@

.PHONY: scripts
scripts: | $(TEST_BUILD_DIR)
	@make --no-print-directory BUILD_DIR=$(TEST_BUILD_DIR) -C scripts
