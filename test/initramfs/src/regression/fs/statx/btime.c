// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#ifndef __asterinas__
#include <linux/loop.h>
#include <sys/ioctl.h>
#endif
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

#define RAMFS_MOUNT_POINT "/statx_btime_ramfs"
#define RAMFS_TEST_FILE RAMFS_MOUNT_POINT "/statx_btime_test_file"

#define EXFAT_MOUNT_POINT "/statx_btime_exfat"
#define EXFAT_TEST_FILE EXFAT_MOUNT_POINT "/statx_btime_test_file"
#define EXFAT_TEST_FILE_NOT_REQ \
	EXFAT_MOUNT_POINT "/statx_btime_test_file_not_req"

#ifndef __asterinas__
#define EXFAT_IMAGE "/tmp/statx_btime_exfat.img"
#define EXFAT_IMAGE_SIZE (64 * 1024 * 1024)
#endif

static int fd = -1;

static int created_ramfs_mount_point;
static int created_exfat_mount_point;

#ifndef __asterinas__
static int loop_fd = -1;
static char loop_path[64];
#endif

static void make_ramfs_mount_point(void)
{
	if (mkdir(RAMFS_MOUNT_POINT, 0755) == 0) {
		created_ramfs_mount_point = 1;
		return;
	}
}

static void make_exfat_mount_point(void)
{
	if (mkdir(EXFAT_MOUNT_POINT, 0755) == 0) {
		created_exfat_mount_point = 1;
		return;
	}
}

#ifndef __asterinas__
static void create_exfat_image(void)
{
	int image_fd =
		CHECK(open(EXFAT_IMAGE, O_CREAT | O_TRUNC | O_RDWR, 0600));

	CHECK(ftruncate(image_fd, EXFAT_IMAGE_SIZE));
	CHECK(close(image_fd));
	CHECK_WITH(system("mkfs.exfat " EXFAT_IMAGE " >/dev/null"), _ret == 0);
}

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
	make_ramfs_mount_point();
	CHECK(mount("none", RAMFS_MOUNT_POINT, "ramfs", 0, NULL));

	fd = CHECK(open(RAMFS_TEST_FILE, O_CREAT | O_RDWR | O_TRUNC, 0644));
	CHECK(write(fd, "test", 4));

	make_exfat_mount_point();
#ifdef __asterinas__
	CHECK(mount("/dev/vdb", EXFAT_MOUNT_POINT, "exfat", 0, ""));
#else
	create_exfat_image();
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
	TEST_RES(0, (stx.stx_mask & STATX_BTIME) == 0);
}
END_TEST()

FN_TEST(statx_btime_ramfs)
{
	struct statx stx;
	TEST_SUCC(syscall(SYS_statx, fd, "", AT_EMPTY_PATH, STATX_BTIME, &stx));
	TEST_RES(0, stx.stx_btime.tv_sec == 0 && stx.stx_btime.tv_nsec == 0);
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
	TEST_RES(0, (stx.stx_mask & STATX_BTIME) != 0);
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
	TEST_RES(0, (stx.stx_mask & STATX_BTIME) != 0);
	TEST_RES(0, stx.stx_btime.tv_sec != 0 || stx.stx_btime.tv_nsec != 0);
	// Verify that the birth time is within the time window around file creation
	TEST_RES(0, stx.stx_btime.tv_sec >= before_create.tv_sec - 1);
	TEST_RES(0, stx.stx_btime.tv_sec <= after_create.tv_sec + 1);
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
	CHECK(unlink(EXFAT_IMAGE));
#endif

	if (created_exfat_mount_point) {
		CHECK(rmdir(EXFAT_MOUNT_POINT));
	}
	if (created_ramfs_mount_point) {
		CHECK(rmdir(RAMFS_MOUNT_POINT));
	}
}
END_SETUP()
