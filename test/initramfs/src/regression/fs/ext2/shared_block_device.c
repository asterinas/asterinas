// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <sched.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"
#include "../../common/block_device.h"

#define PRIMARY_MOUNT "/tmp/shared_block_device_primary"
#define SECONDARY_MOUNT "/tmp/shared_block_device_secondary"
#define TEST_FILE "shared_instance_test"
#define PRIMARY_FILE PRIMARY_MOUNT "/" TEST_FILE
#define SECONDARY_FILE SECONDARY_MOUNT "/" TEST_FILE

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

	CHECK(unshare(CLONE_NEWNS));
}
END_SETUP()

FN_TEST(mounts_of_same_block_device_share_filesystem)
{
	const char *payload = "shared ext2 instance";
	const size_t payload_len = strlen(payload);
	char buffer[sizeof("shared ext2 instance")] = { 0 };

	int block_device_fd = TEST_SUCC(open_block_device(BLOCK_VDC_EXT2));

	/*
	 * Mount the same ext2 block device twice.
	 * Both mount points should refer to one shared filesystem instance.
	 */
	TEST_SUCC(
		mount_block_device(block_device_fd, PRIMARY_MOUNT, "ext2", 0));
	TEST_SUCC(mount_block_device(block_device_fd, SECONDARY_MOUNT, "ext2",
				     0));

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

	TEST_SUCC(umount_block_device(SECONDARY_MOUNT));
	TEST_SUCC(umount_block_device(PRIMARY_MOUNT));
	TEST_SUCC(close_block_device(block_device_fd));
}
END_TEST()

FN_TEST(mounts_of_same_block_device_reject_different_readonly_flags)
{
	int block_device_fd = TEST_SUCC(open_block_device(BLOCK_VDC_EXT2));

	TEST_SUCC(mount_block_device(block_device_fd, PRIMARY_MOUNT, "ext2",
				     MS_RDONLY));
	TEST_ERRNO(mount_block_device(block_device_fd, SECONDARY_MOUNT, "ext2",
				      0),
		   EBUSY);
	TEST_SUCC(umount_block_device(PRIMARY_MOUNT));

	TEST_SUCC(close_block_device(block_device_fd));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(rmdir(PRIMARY_MOUNT));
	CHECK(rmdir(SECONDARY_MOUNT));
}
END_SETUP()
