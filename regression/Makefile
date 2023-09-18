MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
BUILD_DIR := $(CUR_DIR)/build
INITRAMFS := $(BUILD_DIR)/initramfs
INITRAMFS_IMAGE := $(BUILD_DIR)/initramfs.cpio.gz
EXT2_IMAGE := $(BUILD_DIR)/ext2.img
SHELL := /bin/bash
INITRAMFS_EMPTY_DIRS := \
	$(INITRAMFS)/sbin \
	$(INITRAMFS)/root \
	$(INITRAMFS)/tmp \
	$(INITRAMFS)/opt \
	$(INITRAMFS)/proc \
	$(INITRAMFS)/dev \
	$(INITRAMFS)/ext2
INITRAMFS_ALL_DIRS := \
	$(INITRAMFS)/etc \
	$(INITRAMFS)/lib/x86_64-linux-gnu \
	$(INITRAMFS)/lib64 \
	$(INITRAMFS)/bin \
	$(INITRAMFS)/usr/bin \
	$(INITRAMFS)/regression \
	$(INITRAMFS)/benchmark \
	$(INITRAMFS_EMPTY_DIRS)

.PHONY: all clean

all: build

$(INITRAMFS)/lib/x86_64-linux-gnu:
	@mkdir -p $@
	@cp -L /lib/x86_64-linux-gnu/libc.so.6 $@
	@cp -L /lib/x86_64-linux-gnu/libstdc++.so.6 $@
	@cp -L /lib/x86_64-linux-gnu/libm.so.6 $@
	@cp -L /lib/x86_64-linux-gnu/libgcc_s.so.1 $@
	@cp -L /lib/x86_64-linux-gnu/libpthread.so.0 $@
	@# required for benchmarks
	@cp -L /lib/x86_64-linux-gnu/libcrypto.so.3 $@
	@cp -L /lib/x86_64-linux-gnu/libcrypt.so.1 $@
	@cp -L /lib/x86_64-linux-gnu/libdl.so.2 $@
	@cp -L /lib/x86_64-linux-gnu/libz.so.1 $@
	@cp -L /usr/local/benchmark/iperf/lib/libiperf.so.0 $@
	@# TODO: use a custom compiled vdso.so file in the future.
	@git clone https://github.com/asterinas/linux_vdso.git
	@cd ./linux_vdso && git checkout 2a6d2db 2>/dev/null
	@cp -L ./linux_vdso/vdso64.so $@
	@rm -rf ./linux_vdso 


$(INITRAMFS)/lib64:
	@mkdir -p $@
	@cp -L /lib64/ld-linux-x86-64.so.2 $@

$(INITRAMFS)/etc:
	@mkdir -p $@
	@cp $(CUR_DIR)/etc/passwd $@
	@cp $(CUR_DIR)/etc/group $@

# Install busybox into /bin and /usr/bin.
$(INITRAMFS)/bin:
	@mkdir -p $@
	@/bin/busybox --install -s $@

$(INITRAMFS)/usr/bin: $(INITRAMFS)/bin
	@mkdir -p $@
	@cp /usr/bin/busybox $@

# Copy from apps.
$(INITRAMFS)/regression:
	@make --no-print-directory -C apps

$(INITRAMFS)/benchmark:
	@mkdir -p $@
	@cp /usr/local/benchmark/sysbench/bin/sysbench $@
	@cp /usr/local/benchmark/iperf/bin/iperf3 $@
	@cp /usr/local/benchmark/membench/membench $@

# Make necessary directories.
$(INITRAMFS_EMPTY_DIRS):
	@mkdir -p $@

$(INITRAMFS)/opt/syscall_test:
	@make --no-print-directory -C syscall_test

# If the BUILD_SYSCALL_TEST variable is set, we should depend on the
# sub make output to do incremental building.
ifeq ($(BUILD_SYSCALL_TEST), 1)
$(INITRAMFS_IMAGE): $(INITRAMFS_ALL_DIRS) $(INITRAMFS)/opt/syscall_test
else
$(INITRAMFS_IMAGE): $(INITRAMFS_ALL_DIRS)
endif
	@echo "Generating the initramfs image..."
	@(cd $(INITRAMFS); find . | cpio -o -H newc | gzip) > $@

$(EXT2_IMAGE):
	@dd if=/dev/zero of=$(EXT2_IMAGE) bs=2G count=1
	@mke2fs $(EXT2_IMAGE)

build: $(INITRAMFS_IMAGE) $(EXT2_IMAGE)

clean:
	@rm -rf $(BUILD_DIR)
