// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <fcntl.h>
#include <linux/stat.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/time.h>
#include <sys/types.h>
#include <time.h>
#include <unistd.h>

#include "../../common/test.h"

#define RAMFS_MOUNT_POINT "/tmp/statx_btime_ramfs"
#define RAMFS_TEST_FILE RAMFS_MOUNT_POINT "/statx_btime_test_file"

#define EXFAT_MOUNT_POINT "/tmp/statx_btime_exfat"
#define EXFAT_TEST_FILE EXFAT_MOUNT_POINT "/statx_btime_test_file"
#define EXFAT_TEST_FILE_NOT_REQ \
	EXFAT_MOUNT_POINT "/statx_btime_test_file_not_req"

static int fd = -1;

#ifndef __asterinas__
#include <linux/loop.h>
#include <sys/ioctl.h>

#define EXFAT_IMAGE "./test/initramfs/build/exfat.img"

static int loop_fd = -1;
static char loop_path[64];

static void attach_loop_device(void)
{
	int control_fd = CHECK(open("/dev/loop-control", O_RDWR));
	int loop_number = CHECK(ioctl(control_fd, LOOP_CTL_GET_FREE));
	CHECK(close(control_fd));
	CHECK_WITH(snprintf(loop_path, sizeof(loop_path), "/dev/loop%d",
			    loop_number),
		   _ret > 0 && (size_t)_ret < sizeof(loop_path));

	int image_fd = CHECK(open(EXFAT_IMAGE, O_RDWR));
	loop_fd = CHECK(open(loop_path, O_RDWR));
	CHECK(ioctl(loop_fd, LOOP_SET_FD, image_fd));
	CHECK(close(image_fd));
}
#endif

FN_SETUP(prepare)
{
	CHECK_WITH(mkdir(RAMFS_MOUNT_POINT, 0755),
		   _ret == 0 || errno == EEXIST);
	CHECK(mount("none", RAMFS_MOUNT_POINT, "ramfs", 0, NULL));

	fd = CHECK(open(RAMFS_TEST_FILE, O_CREAT | O_RDWR | O_TRUNC, 0644));
	CHECK(write(fd, "test", 4));

	CHECK_WITH(mkdir(EXFAT_MOUNT_POINT, 0755),
		   _ret == 0 || errno == EEXIST);
#ifdef __asterinas__
	CHECK(mount("/dev/vdb", EXFAT_MOUNT_POINT, "exfat", 0, ""));
#else
	attach_loop_device();
	CHECK(mount(loop_path, EXFAT_MOUNT_POINT, "exfat", 0, ""));
#endif
}
END_SETUP()

FN_TEST(statx_btime_not_requested)
{
	struct statx stx;
	TEST_SUCC(syscall(SYS_statx, fd, "", AT_EMPTY_PATH, STATX_BASIC_STATS,
			  &stx));
	TEST_RES((stx.stx_mask & STATX_BTIME) == 0, _ret);
}
END_TEST()

FN_TEST(statx_btime_ramfs)
{
	struct statx stx;
	TEST_SUCC(syscall(SYS_statx, fd, "", AT_EMPTY_PATH, STATX_BTIME, &stx));
	TEST_RES(stx.stx_btime.tv_sec == 0 && stx.stx_btime.tv_nsec == 0, _ret);
}
END_TEST()

FN_TEST(exfat_statx_btime_not_requested)
{
	struct statx stx;
	int test_fd = TEST_SUCC(open(EXFAT_TEST_FILE_NOT_REQ,
				     O_CREAT | O_TRUNC | O_RDWR, 0644));
	TEST_SUCC(syscall(SYS_statx, test_fd, "", AT_EMPTY_PATH,
			  STATX_BASIC_STATS, &stx));
	// exfat supports birth time, so `STATX_BTIME` should be set even if not requested
	TEST_RES((stx.stx_mask & STATX_BTIME) != 0, _ret);
	TEST_SUCC(close(test_fd));
}
END_TEST()

FN_TEST(exfat_statx_reports_birth_time)
{
	struct statx stx;
	struct timespec before_create;
	struct timespec after_create;

	TEST_SUCC(clock_gettime(CLOCK_REALTIME, &before_create));
	int test_fd = TEST_SUCC(
		open(EXFAT_TEST_FILE, O_CREAT | O_TRUNC | O_RDWR, 0644));
	TEST_SUCC(clock_gettime(CLOCK_REALTIME, &after_create));

	memset(&stx, 0, sizeof(stx));
	TEST_RES(write(test_fd, "test", 4), _ret == 4);
	TEST_SUCC(syscall(SYS_statx, test_fd, "", AT_EMPTY_PATH, STATX_BTIME,
			  &stx));
	TEST_RES((stx.stx_mask & STATX_BTIME) != 0, _ret);
	TEST_RES(stx.stx_btime.tv_sec != 0 || stx.stx_btime.tv_nsec != 0, _ret);
	// Verify that the birth time is within the time window around file creation
	TEST_RES(stx.stx_btime.tv_sec >= before_create.tv_sec - 1, _ret);
	TEST_RES(stx.stx_btime.tv_sec <= after_create.tv_sec + 1, _ret);

	TEST_SUCC(close(test_fd));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(fd));
	CHECK(unlink(RAMFS_TEST_FILE));
	CHECK(unlink(EXFAT_TEST_FILE));
	CHECK(unlink(EXFAT_TEST_FILE_NOT_REQ));
	CHECK(umount(EXFAT_MOUNT_POINT));
	CHECK(umount(RAMFS_MOUNT_POINT));

#ifndef __asterinas__
	CHECK(ioctl(loop_fd, LOOP_CLR_FD));
	CHECK(close(loop_fd));
#endif

	CHECK(rmdir(EXFAT_MOUNT_POINT));
	CHECK(rmdir(RAMFS_MOUNT_POINT));
}
END_SETUP()
