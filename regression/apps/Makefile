MAKEFLAGS += --no-builtin-rules # Prevent the implicit rules from compiling ".c" or ".s" files automatically.
MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
INITRAMFS ?= $(CUR_DIR)/../build/initramfs

.PHONY: all

all: build

build:
	@cp -a $(CUR_DIR)/* $(INITRAMFS)
	@cd $(INITRAMFS) && find . \( -name "*.s" -o -name "*.c" -o -name "Makefile" -o -name "README.md" \) -delete
