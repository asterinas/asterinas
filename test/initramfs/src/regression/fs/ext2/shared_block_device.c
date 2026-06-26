// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#ifndef __asterinas__
#include <linux/loop.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/ioctl.h>
#endif
#include <sched.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"

#define PRIMARY_MOUNT "/tmp/shared_block_device_primary"
#define SECONDARY_MOUNT "/tmp/shared_block_device_secondary"
#define TEST_FILE "shared_instance_test"
#define PRIMARY_FILE PRIMARY_MOUNT "/" TEST_FILE
#define SECONDARY_FILE SECONDARY_MOUNT "/" TEST_FILE

#ifdef __asterinas__
#define BLOCK_DEVICE "/dev/vda"

static const char *block_device_path(void)
{
	return BLOCK_DEVICE;
}

static int open_block_device(void)
{
	return open(BLOCK_DEVICE, O_RDWR);
}

static int close_block_device(int fd)
{
	return close(fd);
}
#else
#define LOOP_BACKING_FILE "/tmp/shared_block_device.img"
#define LOOP_BACKING_FILE_SIZE (16 * 1024 * 1024)

static char loop_path[sizeof("/dev/loop") + 10];

static const char *block_device_path(void)
{
	return loop_path;
}

static void make_ext2_image(void)
{
	/*
	 * Linux test runners do not have the Asterinas ext2 disk image, so
	 * build a fresh loop-backed ext2 block device for the same mount path.
	 */
	int image_fd = CHECK(
		open(LOOP_BACKING_FILE, O_CREAT | O_RDWR | O_TRUNC, 0600));

	CHECK(ftruncate(image_fd, LOOP_BACKING_FILE_SIZE));
	CHECK(close(image_fd));
	CHECK_WITH(system("mke2fs -q -t ext2 -F " LOOP_BACKING_FILE),
		   _ret == 0);
}

static int open_block_device(void)
{
	make_ext2_image();

	/* Keep the loop fd open until both mounts are unmounted. */
	int control_fd = CHECK(open("/dev/loop-control", O_RDWR));
	int loop_number = CHECK(ioctl(control_fd, LOOP_CTL_GET_FREE));
	CHECK(close(control_fd));

	snprintf(loop_path, sizeof(loop_path), "/dev/loop%d", loop_number);
	int loop_fd = CHECK(open(loop_path, O_RDWR));
	int image_fd = CHECK(open(LOOP_BACKING_FILE, O_RDWR));
	CHECK(ioctl(loop_fd, LOOP_SET_FD, image_fd));
	CHECK(close(image_fd));
	return loop_fd;
}

static int close_block_device(int fd)
{
	CHECK(ioctl(fd, LOOP_CLR_FD, 0));
	CHECK(close(fd));
	return unlink(LOOP_BACKING_FILE);
}
#endif

static void ensure_dir(const char *path)
{
	CHECK_WITH(mkdir(path, 0755), _ret >= 0 || errno == EEXIST);
}

static void unlink_if_exists(const char *path)
{
	CHECK_WITH(unlink(path), _ret == 0 || errno == ENOENT);
}

FN_SETUP(create_mount_dirs)
{
	ensure_dir(PRIMARY_MOUNT);
	ensure_dir(SECONDARY_MOUNT);
}
END_SETUP()

FN_TEST(mounts_of_same_block_device_share_filesystem)
{
	const char *payload = "shared ext2 instance";
	const size_t payload_len = strlen(payload);
	char buffer[sizeof("shared ext2 instance")] = { 0 };
	struct stat source_stat;

	int block_device_fd = TEST_SUCC(open_block_device());
	const char *block_device = block_device_path();
	TEST_RES(stat(block_device, &source_stat),
		 S_ISBLK(source_stat.st_mode));

	/*
	 * Mount the same ext2 block device twice in a private mount namespace.
	 * Both mount points should refer to one shared filesystem instance.
	 */
	TEST_SUCC(unshare(CLONE_NEWNS));
	TEST_SUCC(mount(block_device, PRIMARY_MOUNT, "ext2", 0, ""));
	TEST_SUCC(mount(block_device, SECONDARY_MOUNT, "ext2", 0, ""));

	unlink_if_exists(PRIMARY_FILE);
	TEST_ERRNO(open(SECONDARY_FILE, O_RDONLY), ENOENT);

	/* A file created from the primary mount must be visible from the other. */
	int file_fd = TEST_SUCC(
		open(PRIMARY_FILE, O_CREAT | O_EXCL | O_WRONLY, 0644));
	TEST_RES(write(file_fd, payload, payload_len),
		 _ret == (ssize_t)payload_len);
	TEST_SUCC(close(file_fd));

	file_fd = TEST_SUCC(open(SECONDARY_FILE, O_RDONLY));
	TEST_RES(read(file_fd, buffer, sizeof(buffer)),
		 _ret == (ssize_t)payload_len);
	TEST_RES(memcmp(buffer, payload, payload_len), _ret == 0);
	TEST_SUCC(close(file_fd));

	/* Removing the file from the secondary mount must remove the same inode. */
	TEST_SUCC(unlink(SECONDARY_FILE));
	TEST_ERRNO(open(PRIMARY_FILE, O_RDONLY), ENOENT);

	TEST_SUCC(umount(SECONDARY_MOUNT));
	TEST_SUCC(umount(PRIMARY_MOUNT));
	TEST_SUCC(close_block_device(block_device_fd));
}
END_TEST()
