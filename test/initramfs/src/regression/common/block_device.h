/* SPDX-License-Identifier: MPL-2.0 */

#ifndef BLOCK_DEVICE_H
#define BLOCK_DEVICE_H

/*
 * Utilities to operate block devices in guest Asterinas, Linux regression VMs,
 * and host Linux.
 *
 * By opening a block device via open_block_device(), the device is opened via
 * /dev/vda (or similar) in guest Asterinas and in Linux builds compiled with
 * __linux_vm__, or via /dev/loop0 (or similar) on host Linux.
 * These correspond to the same disk image prepared as part of the build
 * process.
 *
 * The device can be mounted using mount_block_device(). After use, the mount
 * can be cleaned up and the device can be closed via umount_block_device() and
 * close_block_device(), respectively.
 */

#include <sys/mount.h>
#include <stdio.h>
#include <fcntl.h>
#include <unistd.h>

enum block_device {
	BLOCK_VDA_EXT2,
	BLOCK_VDB_EXFAT,
	BLOCK_VDC_EXT2,
};

#if defined(__asterinas__) || defined(__linux_vm__)

static int open_block_device(enum block_device dev)
{
	switch (dev) {
	case BLOCK_VDA_EXT2:
		return open("/dev/vda", O_RDWR);
	case BLOCK_VDB_EXFAT:
		return open("/dev/vdb", O_RDWR);
	case BLOCK_VDC_EXT2:
		return open("/dev/vdc", O_RDWR);
	}

	errno = ENODEV;
	return -1;
}

static int close_block_device(int fd)
{
	return close(fd);
}

#else /* __asterinas__ || __linux_vm__ */

#include <sys/ioctl.h>
#include <linux/loop.h>

#include "test.h"

static int open_block_device(enum block_device dev)
{
	int img_fd, ctrl_fd, loop_fd;
	int id;
	char loop_path[20];

	img_fd = -1;
	switch (dev) {
	case BLOCK_VDA_EXT2:
		img_fd = CHECK(open("./test/initramfs/build/ext2.img", O_RDWR));
		break;
	case BLOCK_VDB_EXFAT:
		img_fd =
			CHECK(open("./test/initramfs/build/exfat.img", O_RDWR));
		break;
	case BLOCK_VDC_EXT2:
		img_fd = CHECK(
			open("./test/initramfs/build/ltp_dev.img", O_RDWR));
		break;
	}

	ctrl_fd = CHECK(open("/dev/loop-control", O_RDWR));
	id = CHECK(ioctl(ctrl_fd, LOOP_CTL_GET_FREE));
	CHECK(close(ctrl_fd));

	snprintf(loop_path, sizeof(loop_path), "/dev/loop%d", id);
	loop_fd = CHECK(open(loop_path, O_RDWR));
	CHECK(ioctl(loop_fd, LOOP_SET_FD, img_fd));
	CHECK(close(img_fd));

	return loop_fd;
}

static int close_block_device(int fd)
{
	CHECK(ioctl(fd, LOOP_CLR_FD));
	CHECK(close(fd));

	return 0;
}

#endif /* __asterinas__ || __linux_vm__ */

static int __attribute__((unused)) mount_block_device(int fd, const char *path,
						      const char *fs, int flags)
{
	char buf[25];
	snprintf(buf, sizeof(buf), "/proc/self/fd/%d", fd);

	return mount(buf, path, fs, flags, NULL);
}

static int __attribute__((unused)) umount_block_device(const char *path)
{
	return umount(path);
}

#endif /* BLOCK_DEVICE_H */
