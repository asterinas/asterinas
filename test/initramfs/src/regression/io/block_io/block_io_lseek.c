// SPDX-License-Identifier: MPL-2.0

#include <errno.h>
#include <fcntl.h>
#include <linux/fs.h>
#ifndef __asterinas__
#include <linux/loop.h>
#endif
#include <stdio.h>
#include <stdlib.h>
#include <sys/ioctl.h>
#include <unistd.h>
#include <stdint.h>

#include "../../common/test.h"

#ifndef __asterinas__
#define TEST_BLOCK_DEVICE_SIZE 8192
#define TEST_BACKING_FILE_TEMPLATE "/tmp/block_io_lseek.XXXXXX"

static char test_backing_file[] = TEST_BACKING_FILE_TEMPLATE;
static char test_block_device[32];
static int loop_device_attached;

static int should_skip_loop_device_setup(int err)
{
	return err == EACCES || err == ENODEV || err == ENOENT ||
	       err == ENXIO || err == EPERM;
}

static void cleanup_loop_device(void)
{
	if (loop_device_attached) {
		int loop_fd = open(test_block_device, O_RDWR);

		if (loop_fd >= 0) {
			ioctl(loop_fd, LOOP_CLR_FD, 0);
			close(loop_fd);
		}

		loop_device_attached = 0;
	}

	unlink(test_backing_file);
}

static void skip_loop_device_test(const char *operation, int err)
{
	cleanup_loop_device();
	fprintf(stderr, "block_io_lseek skipped: %s failed: %s\n", operation,
		strerror(err));
	exit(EXIT_SUCCESS);
}

static void fail_loop_device_setup(const char *operation)
{
	fprintf(stderr, "fatal error: prepare_block_device: %s failed: %s\n",
		operation, strerror(errno));
	exit(EXIT_FAILURE);
}

FN_SETUP(prepare_block_device)
{
	int backing_fd;
	int loop_control_fd;
	int loop_device_number;
	int loop_fd;
	int path_len;

	backing_fd = mkstemp(test_backing_file);
	if (backing_fd < 0) {
		fail_loop_device_setup("mkstemp()");
	}

	if (ftruncate(backing_fd, TEST_BLOCK_DEVICE_SIZE) < 0) {
		close(backing_fd);
		fail_loop_device_setup("ftruncate()");
	}

	if (atexit(cleanup_loop_device) != 0) {
		close(backing_fd);
		errno = ENOMEM;
		fail_loop_device_setup("atexit()");
	}

	loop_control_fd = open("/dev/loop-control", O_RDWR);
	if (loop_control_fd < 0) {
		int err = errno;

		close(backing_fd);
		if (should_skip_loop_device_setup(err)) {
			skip_loop_device_test("open('/dev/loop-control')", err);
		}
		errno = err;
		fail_loop_device_setup("open('/dev/loop-control')");
	}

	loop_device_number = ioctl(loop_control_fd, LOOP_CTL_GET_FREE);
	if (loop_device_number < 0) {
		int err = errno;

		close(loop_control_fd);
		close(backing_fd);
		if (should_skip_loop_device_setup(err)) {
			skip_loop_device_test("ioctl(LOOP_CTL_GET_FREE)", err);
		}
		errno = err;
		fail_loop_device_setup("ioctl(LOOP_CTL_GET_FREE)");
	}

	path_len = snprintf(test_block_device, sizeof(test_block_device),
			    "/dev/loop%d", loop_device_number);
	if (path_len < 0 || path_len >= (int)sizeof(test_block_device)) {
		close(loop_control_fd);
		close(backing_fd);
		errno = ENAMETOOLONG;
		fail_loop_device_setup("snprintf()");
	}

	loop_fd = open(test_block_device, O_RDWR);
	if (loop_fd < 0) {
		int err = errno;

		close(loop_control_fd);
		close(backing_fd);
		if (should_skip_loop_device_setup(err)) {
			skip_loop_device_test("open(loop device)", err);
		}
		errno = err;
		fail_loop_device_setup("open(loop device)");
	}

	if (ioctl(loop_fd, LOOP_SET_FD, backing_fd) < 0) {
		int err = errno;

		close(loop_fd);
		close(loop_control_fd);
		close(backing_fd);
		if (should_skip_loop_device_setup(err)) {
			skip_loop_device_test("ioctl(LOOP_SET_FD)", err);
		}
		errno = err;
		fail_loop_device_setup("ioctl(LOOP_SET_FD)");
	}

	loop_device_attached = 1;

	CHECK(close(loop_fd));
	CHECK(close(loop_control_fd));
	CHECK(close(backing_fd));
}
END_SETUP()

#define DEVICE test_block_device
#else
#define DEVICE "/dev/vda"
#endif

FN_TEST(block_lseek)
{
	int fd = TEST_SUCC(open(DEVICE, O_RDONLY));

	// Get device size via BLKGETSIZE64
	uint64_t blk_size = 0;
	TEST_SUCC(ioctl(fd, BLKGETSIZE64, &blk_size));

	// SEEK_SET
	TEST_RES(lseek(fd, 1234, SEEK_SET), _ret == 1234);

	// SEEK_CUR after SEEK_SET
	TEST_RES(lseek(fd, 0, SEEK_CUR), _ret == 1234);

	// SEEK_END should equal BLKGETSIZE64
	TEST_RES(lseek(fd, 0, SEEK_END), (uint64_t)_ret == blk_size);

	// SEEK_CUR after SEEK_END
	TEST_RES(lseek(fd, 0, SEEK_CUR), (uint64_t)_ret == blk_size);

	// SEEK_END with negative offset
	TEST_RES(lseek(fd, -512, SEEK_END), (uint64_t)_ret == blk_size - 512);

	// SEEK_CUR with positive offset
	TEST_RES(lseek(fd, 256, SEEK_CUR), (uint64_t)_ret == blk_size - 256);

	// SEEK_SET back to 0
	TEST_RES(lseek(fd, 0, SEEK_SET), _ret == 0);

	TEST_SUCC(close(fd));
}
END_TEST()
