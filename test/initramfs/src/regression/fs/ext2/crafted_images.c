// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"
#include "../../common/block_device.h"

/*
 * Regression cases backed by malformed ext2 images. The images are produced
 * at test setup time by ../../../../../tools/make_fault_images.sh from a
 * clean mke2fs base; no image binary is committed.
 *
 * Both cases are marked draft upstream: the fixes are not merged yet. With
 * the current kernel, each case panics the guest and the test never returns.
 */

#define BAD_ROOT_MOUNT "/tmp/ext2_bad_root"
#define BAD_IND_MOUNT "/tmp/ext2_bad_indirect"

FN_SETUP(prepare_mount_dirs)
{
	CHECK_WITH(mkdir(BAD_ROOT_MOUNT, 0755), _ret == 0 || errno == EEXIST);
	CHECK_WITH(mkdir(BAD_IND_MOUNT, 0755), _ret == 0 || errno == EEXIST);
}
END_SETUP()

/* #3866: a deleted root inode must fail the mount, not panic. */
FN_TEST(unresolvable_root_inode_mount_fails)
{
	int fd = TEST_SUCC(open_block_device(BLOCK_VDC_EXT2));
	/* The test runner attaches bad_root.img as this device. */
	TEST_ERRNO(mount_block_device(fd, BAD_ROOT_MOUNT, "ext2", 0),
		   EUCLEAN);
	TEST_SUCC(close_block_device(fd));
}
END_TEST()

/* #4128: an out-of-range indirect pointer must fail both I/O directions. */
FN_TEST(invalid_indirect_block_access_fails)
{
	int fd = TEST_SUCC(open_block_device(BLOCK_VDC_EXT2));
	/* The test runner attaches bad_indirect.img as this device. */
	TEST_SUCC(mount_block_device(fd, BAD_IND_MOUNT, "ext2", 0));

	int f = TEST_SUCC(open(BAD_IND_MOUNT "/ind2", O_RDWR));
	char buf[64];
	TEST_ERRNO(pread(f, buf, sizeof(buf), 12 * 4096), EIO);
	TEST_ERRNO(pwrite(f, "Z", 1, 12 * 4096), EIO);
	TEST_SUCC(close(f));

	TEST_SUCC(umount(BAD_IND_MOUNT));
	TEST_SUCC(close_block_device(fd));
}
END_TEST()
