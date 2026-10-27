// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <sched.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/xattr.h>
#include <unistd.h>

#include "../../common/test.h"
#include "../../common/block_device.h"

#define MOUNT_POINT "/tmp/superblock_lifecycle"
#define TEST_FILE MOUNT_POINT "/final_writeback"
#define TEST_BLOCK_SIZE 4096
#define DATA_OFFSET (13 * TEST_BLOCK_SIZE)

FN_SETUP(setup)
{
	CHECK(unshare(CLONE_NEWNS));
	CHECK(mkdir(MOUNT_POINT, 0755));
}
END_SETUP()

FN_TEST(final_unmount_after_explicit_sync)
{
	int device = TEST_SUCC(open_block_device(BLOCK_VDC_EXT2));
	TEST_SUCC(mount_block_device(device, MOUNT_POINT, "ext2", 0));
	int fd = TEST_SUCC(open(TEST_FILE, O_CREAT | O_EXCL | O_RDWR, 0600));
	const char payload[] = "explicitly synced data";
	TEST_RES(write(fd, payload, sizeof(payload)), _ret == sizeof(payload));
	TEST_SUCC(fsync(fd));
	TEST_SUCC(close(fd));
	TEST_SUCC(umount_block_device(MOUNT_POINT));

	TEST_SUCC(mount_block_device(device, MOUNT_POINT, "ext2", 0));
	fd = TEST_SUCC(open(TEST_FILE, O_RDONLY));
	char actual[sizeof(payload)];
	TEST_RES(read(fd, actual, sizeof(actual)), _ret == sizeof(actual));
	TEST_RES(memcmp(actual, payload, sizeof(payload)), _ret == 0);
	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(TEST_FILE));
	TEST_SUCC(umount_block_device(MOUNT_POINT));
	TEST_SUCC(close_block_device(device));
}
END_TEST()

FN_TEST(final_close_after_lazy_unmount)
{
	int device = TEST_SUCC(open_block_device(BLOCK_VDC_EXT2));
	TEST_SUCC(mount_block_device(device, MOUNT_POINT, "ext2", 0));
	int fd = TEST_SUCC(open(TEST_FILE, O_CREAT | O_EXCL | O_RDWR, 0600));
	TEST_SUCC(umount2(MOUNT_POINT, MNT_DETACH));
	TEST_ERRNO(open(TEST_FILE, O_RDONLY), ENOENT);

	/* Dirty data, indirect blocks, inode size and xattrs after detach. */
	const char payload[] = "written after lazy unmount";
	TEST_RES(pwrite(fd, payload, sizeof(payload), DATA_OFFSET),
		 _ret == sizeof(payload));
	TEST_SUCC(fsetxattr(fd, "user.final_release", payload, sizeof(payload),
			    0));
	TEST_SUCC(close(fd));

	TEST_SUCC(mount_block_device(device, MOUNT_POINT, "ext2", 0));
	fd = TEST_SUCC(open(TEST_FILE, O_RDONLY));
	struct stat st;
	TEST_SUCC(fstat(fd, &st));
	TEST_RES(st.st_size, _ret == DATA_OFFSET + sizeof(payload));
	char actual[sizeof(payload)];
	TEST_RES(pread(fd, actual, sizeof(actual), DATA_OFFSET),
		 _ret == sizeof(actual));
	TEST_RES(memcmp(actual, payload, sizeof(payload)), _ret == 0);
	TEST_RES(fgetxattr(fd, "user.final_release", actual, sizeof(actual)),
		 _ret == sizeof(actual));
	TEST_RES(memcmp(actual, payload, sizeof(payload)), _ret == 0);
	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(TEST_FILE));
	TEST_SUCC(umount_block_device(MOUNT_POINT));
	TEST_SUCC(close_block_device(device));
}
END_TEST()

FN_TEST(mapping_keeps_detached_filesystem_alive)
{
	int device = TEST_SUCC(open_block_device(BLOCK_VDC_EXT2));
	TEST_SUCC(mount_block_device(device, MOUNT_POINT, "ext2", 0));
	int fd = TEST_SUCC(open(TEST_FILE, O_CREAT | O_EXCL | O_RDWR, 0600));
	TEST_SUCC(ftruncate(fd, TEST_BLOCK_SIZE));
	char *mapping =
		TEST_RES(mmap(NULL, TEST_BLOCK_SIZE, PROT_READ | PROT_WRITE,
			      MAP_SHARED, fd, 0),
			 _ret != MAP_FAILED);
	TEST_SUCC(close(fd));
	TEST_SUCC(umount2(MOUNT_POINT, MNT_DETACH));

	const char payload[] = "mapped write after final close and detach";
	memcpy(mapping, payload, sizeof(payload));
	TEST_SUCC(munmap(mapping, TEST_BLOCK_SIZE));

	TEST_SUCC(mount_block_device(device, MOUNT_POINT, "ext2", 0));
	fd = TEST_SUCC(open(TEST_FILE, O_RDONLY));
	char actual[sizeof(payload)];
	TEST_RES(read(fd, actual, sizeof(actual)), _ret == sizeof(actual));
	TEST_RES(memcmp(actual, payload, sizeof(payload)), _ret == 0);
	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(TEST_FILE));
	TEST_SUCC(umount_block_device(MOUNT_POINT));
	TEST_SUCC(close_block_device(device));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(rmdir(MOUNT_POINT));
}
END_SETUP()
