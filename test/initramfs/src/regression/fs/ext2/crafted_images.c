// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"
#include "../../common/block_device.h"

/*
 * Regression cases for malformed on-disk state, built inside the test.
 *
 * The block device handed in is a clean ext2 image. Each test rewrites the
 * few bytes it needs on that clean device, mounts it, and asserts that the
 * kernel reports an error instead of panicking. Nothing is prepared at build
 * time and no image is committed, so the test runs against any ext2 fixture
 * the runner provides.
 *
 * Each case saves the bytes it overwrites and restores them before it
 * returns, so the two cases do not depend on execution order.
 *
 * Before the corresponding fixes, each case panics the guest instead of
 * returning, which is what makes it red.
 */

#define BAD_ROOT_MOUNT "/tmp/ext2_bad_root"
#define BAD_IND_MOUNT "/tmp/ext2_bad_indirect"

#define FS_BLOCK_SIZE 4096
#define INDIRECT_INDEX 12 /* i_block[12] holds the single-indirect pointer */

/* Group descriptor 0 starts at block 1 with 4 KiB blocks; bg_inode_table is
 * its third u32. Pointing it at block 2, the block bitmap, makes inode-table
 * reads see a zero link count for the root inode. */
#define BG_INODE_TABLE_OFFSET 4104

static uint32_t get_u32(const unsigned char *b, off_t o)
{
	uint32_t v;

	memcpy(&v, b + o, sizeof(v));
	return v;
}

static uint16_t get_u16(const unsigned char *b, off_t o)
{
	uint16_t v;

	memcpy(&v, b + o, sizeof(v));
	return v;
}

/*
 * Byte offset of a single-indirect block in `dev`, read from the inode of
 * `ino`. The test created that file itself, so the layout is the one ext2
 * assigns to a file of thirteen 4 KiB blocks.
 */
static off_t indirect_block_offset(int dev, uint32_t ino, unsigned long *bs_out)
{
	unsigned char sb[1024];
	unsigned long bs;
	uint32_t ipg, first_data_block, group, index, inode_table;
	uint16_t inode_size, desc_size;
	unsigned char desc[128];
	uint32_t ind;

	if (pread(dev, sb, sizeof(sb), 1024) != (ssize_t)sizeof(sb))
		return -1;

	bs = 1024UL << get_u32(sb, 0x18);
	ipg = get_u32(sb, 0x28);
	first_data_block = get_u32(sb, 0x14);
	inode_size = get_u16(sb, 0x58);
	desc_size = get_u16(sb, 0xF0);
	if (desc_size != 32 && desc_size != 64 && desc_size != 128)
		desc_size = bs == 1024 ? 32 : 64;

	group = (ino - 1) / ipg;
	index = (ino - 1) % ipg;

	if (pread(dev, desc, desc_size,
		  (off_t)(first_data_block + 1) * bs +
			  (off_t)group * desc_size) != (ssize_t)desc_size)
		return -1;
	inode_table = get_u32(desc, 8);

	off_t inode_off = (off_t)inode_table * bs + (off_t)index * inode_size;
	if (pread(dev, &ind, sizeof(ind),
		  inode_off + 0x28 + INDIRECT_INDEX * 4) !=
	    (ssize_t)sizeof(ind))
		return -1;

	if (bs_out)
		*bs_out = bs;
	return (off_t)ind * bs;
}

FN_SETUP(prepare_mount_dirs)
{
	CHECK_WITH(mkdir(BAD_ROOT_MOUNT, 0755), _ret == 0 || errno == EEXIST);
	CHECK_WITH(mkdir(BAD_IND_MOUNT, 0755), _ret == 0 || errno == EEXIST);
}
END_SETUP()

/* #3866: an unresolvable root inode must fail the mount, not panic. */
FN_TEST(unresolvable_root_inode_mount_fails)
{
	int dev = TEST_SUCC(open_block_device(BLOCK_VDC_EXT2));

	/* The image must be usable before it is corrupted, otherwise the mount
	 * failure below would prove nothing. */
	TEST_SUCC(mount_block_device(dev, BAD_ROOT_MOUNT, "ext2", 0));
	TEST_SUCC(umount(BAD_ROOT_MOUNT));

	uint32_t original;
	TEST_RES(pread(dev, &original, sizeof(original), BG_INODE_TABLE_OFFSET),
		 _ret == (ssize_t)sizeof(original));

	uint32_t bitmap_block = 2;
	TEST_RES(pwrite(dev, &bitmap_block, sizeof(bitmap_block),
			BG_INODE_TABLE_OFFSET),
		 _ret == (ssize_t)sizeof(bitmap_block));
	TEST_SUCC(fsync(dev));

	TEST_ERRNO(mount_block_device(dev, BAD_ROOT_MOUNT, "ext2", 0), EUCLEAN);

	/* Restore the field so the next case starts from a clean image. */
	TEST_RES(pwrite(dev, &original, sizeof(original),
			BG_INODE_TABLE_OFFSET),
		 _ret == (ssize_t)sizeof(original));
	TEST_SUCC(fsync(dev));
	TEST_SUCC(close_block_device(dev));
}
END_TEST()

/* #4128: an out-of-range indirect pointer must fail both directions. */
FN_TEST(invalid_indirect_block_access_fails)
{
	int dev = TEST_SUCC(open_block_device(BLOCK_VDC_EXT2));
	TEST_SUCC(mount_block_device(dev, BAD_IND_MOUNT, "ext2", 0));

	/* Thirteen blocks exceed the twelve direct pointers, so the file owns a
	 * single-indirect block. */
	const char *path = BAD_IND_MOUNT "/ind2";
	int f = TEST_SUCC(open(path, O_CREAT | O_RDWR, 0644));
	unsigned char block[FS_BLOCK_SIZE];

	memset(block, 'i', sizeof(block));
	for (int i = 0; i < 13; i++)
		TEST_RES(pwrite(f, block, sizeof(block),
				(off_t)i * FS_BLOCK_SIZE),
			 _ret == (ssize_t)sizeof(block));

	struct stat st;
	TEST_SUCC(fstat(f, &st));
	TEST_SUCC(close(f));
	TEST_SUCC(umount(BAD_IND_MOUNT));

	unsigned long bs = 0;
	off_t ind = indirect_block_offset(dev, (uint32_t)st.st_ino, &bs);
	TEST_RES(ind, ind > 0);

	unsigned char saved[FS_BLOCK_SIZE];
	TEST_RES(pread(dev, saved, sizeof(saved), ind),
		 _ret == (ssize_t)sizeof(saved));

	unsigned char poison[FS_BLOCK_SIZE];
	memset(poison, 0xff, sizeof(poison));
	TEST_RES(pwrite(dev, poison, sizeof(poison), ind),
		 _ret == (ssize_t)sizeof(poison));
	TEST_SUCC(fsync(dev));

	/* Reads and writes of that logical block must report an error, and the
	 * filesystem must stay usable afterwards. */
	TEST_SUCC(mount_block_device(dev, BAD_IND_MOUNT, "ext2", 0));
	f = TEST_SUCC(open(path, O_RDWR));

	unsigned char buf[64];
	TEST_ERRNO(pread(f, buf, sizeof(buf),
			 (off_t)INDIRECT_INDEX * FS_BLOCK_SIZE),
		   EIO);
	TEST_RES(pwrite(f, "Z", 1, (off_t)INDIRECT_INDEX * FS_BLOCK_SIZE),
		 _ret < 0 && (errno == EIO || errno == EUCLEAN));
	TEST_SUCC(close(f));
	TEST_SUCC(umount(BAD_IND_MOUNT));

	/* Restore the block so the next case starts from a clean image. */
	TEST_RES(pwrite(dev, saved, sizeof(saved), ind),
		 _ret == (ssize_t)sizeof(saved));
	TEST_SUCC(fsync(dev));
	TEST_SUCC(close_block_device(dev));
}
END_TEST()
