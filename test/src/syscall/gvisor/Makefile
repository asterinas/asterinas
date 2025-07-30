# SPDX-License-Identifier: MPL-2.0

# A list of enabled system call test suites from Gvisor.
# Each test suite consists of multiple test cases,
# some of which are disabled by the blocklists.
#
# Please keep the list sorted by name.
TESTS ?= \
	access_test \
	chown_test \
	creat_test \
	dup_test \
	epoll_test \
	eventfd_test \
	fallocate_test \
	fcntl_test \
	flock_test \
	fsync_test \
	futex_test \
	getdents_test \
	ioctl_test \
	link_test \
	lseek_test \
	memfd_test \
	mkdir_test \
	mknod_test \
	mmap_test \
	mount_test \
	mremap_test \
	msync_test \
	open_create_test \
	open_test \
	ppoll_test \
	prctl_setuid_test \
	pread64_test \
	preadv2_test \
	proc_test \
	pselect_test \
	pty_test \
	pwrite64_test \
	pwritev2_test \
	read_test \
	readv_test \
	rename_test \
	rlimits_test \
	sched_test \
	sched_yield_test \
	semaphore_test \
	sendfile_test \
	sigaction_test \
	sigaltstack_test \
	signalfd_test \
	socket_netlink_route_test \
	socket_unix_pair_test \
	socket_unix_seqpacket_local_test \
	socket_unix_stream_test \
	socket_unix_unbound_seqpacket_test \
	socket_unix_unbound_stream_test \
	stat_test \
	stat_times_test \
	statfs_test \
	symlink_test \
	sync_test \
	sysinfo_test \
	tcp_socket_test \
	timerfd_test \
	timers_test \
	truncate_test \
	uidgid_test \
	unlink_test \
	utimes_test \
	vdso_clock_gettime_test \
	vfork_test \
	write_test \
	xattr_test \
	# The end of the list

MKFILE_PATH := $(abspath $(lastword $(MAKEFILE_LIST)))
CUR_DIR := $(patsubst %/,%,$(dir $(MKFILE_PATH)))
BUILD_DIR ?= $(CUR_DIR)/../../../build
ifdef GVISOR_PREBUILT_DIR
	BIN_DIR := $(GVISOR_PREBUILT_DIR)
else
	BIN_DIR := $(BUILD_DIR)/syscall_test_bins
	SRC_DIR := $(BUILD_DIR)/gvisor_src
endif
INITRAMFS ?= $(CUR_DIR)/../../../build/initramfs
TARGET_DIR := $(INITRAMFS)/opt/gvisor
RUN_BASH := $(CUR_DIR)/run_gvisor_test.sh
BLOCK_LIST := $(CUR_DIR)/blocklists
EXFAT_BLOCK_LIST := $(CUR_DIR)/blocklists.exfat

.PHONY: all
all: $(TESTS)

$(TESTS): $(BIN_DIR) $(TARGET_DIR)
	@cp -f $</$@ $(TARGET_DIR)/tests

ifndef GVISOR_PREBUILT_DIR
$(BIN_DIR): $(SRC_DIR)
	@if ! type bazel > /dev/null; then \
		echo "bazel is not installed, please run tools/install_bazel.sh with sudo permission to install it."; \
		exit 1; \
	fi
	@rm -rf $@ && mkdir -p $@
	@cd $(SRC_DIR) && bazel build --test_tag_filters=native //test/syscalls/...
	@cp $(SRC_DIR)/bazel-bin/test/syscalls/linux/*_test $@

$(SRC_DIR):
	@rm -rf $@ && mkdir -p $@
	@cd $@ && git clone -b 20200921.0 https://github.com/asterinas/gvisor.git .
endif

$(TARGET_DIR): $(RUN_BASH) $(BLOCK_LIST) $(EXFAT_BLOCK_LIST)
	@rm -rf $@ && mkdir -p $@
	@# Prepare tests dir for test binaries
	@mkdir $@/tests
	@# Copy blocklists
	@cp -rf $(BLOCK_LIST) $@
	@# Copy exFAT specific blocklists
	@cp -rf $(EXFAT_BLOCK_LIST) $@
	@# Copy bash script
	@cp -f $(RUN_BASH) $@

.PHONY: clean
clean:
	@rm -rf $(TARGET_DIR)
