// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"
#include <fcntl.h>
#include <linux/fs.h>
#include <stdint.h>
#include <sys/ioctl.h>
#include <unistd.h>

#define SECTOR_SIZE 512

#ifdef __asterinas__
static int open_block_device(void)
{
	return open("/dev/vdb", O_RDWR);
}

static int close_block_device(int fd)
{
	return close(fd);
}
#else
#include <linux/loop.h>

#define LOOP_BACKING_FILE "/tmp/block_device_file_io.img"
#define LOOP_BACKING_FILE_SIZE (SECTOR_SIZE * 8)

static int open_block_device(void)
{
	int control_fd = CHECK(open("/dev/loop-control", O_RDWR));
	int image_fd = CHECK(
		open(LOOP_BACKING_FILE, O_CREAT | O_RDWR | O_TRUNC, 0600));
	int loop_number;
	char loop_path[sizeof("/dev/loop") + 10];

	CHECK(ftruncate(image_fd, LOOP_BACKING_FILE_SIZE));
	loop_number = CHECK(ioctl(control_fd, LOOP_CTL_GET_FREE));
	CHECK(close(control_fd));

	snprintf(loop_path, sizeof(loop_path), "/dev/loop%d", loop_number);
	int loop_fd = CHECK(open(loop_path, O_RDWR));
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

// Verifies that seeking to the end of a block device reports the same
// byte-granular size that the block-device ioctl reports.
FN_TEST(seek_end_matches_block_device_size)
{
	int fd;
	uint64_t block_device_size;

	fd = TEST_SUCC(open_block_device());

	TEST_SUCC(ioctl(fd, BLKGETSIZE64, &block_device_size));
	TEST_RES(lseek(fd, 0, SEEK_END), (uint64_t)_ret == block_device_size);
	TEST_RES(lseek(fd, -SECTOR_SIZE, SEEK_END),
		 (uint64_t)_ret == block_device_size - SECTOR_SIZE);

	TEST_SUCC(close_block_device(fd));
}
END_TEST()

// Verifies that short and non-sector-aligned block-device reads return the
// requested byte range instead of leaking sector-sized read internals.
FN_TEST(short_unaligned_pread_matches_sector_bytes)
{
	int fd;
	uint8_t reference[SECTOR_SIZE * 2];
	uint8_t short_read[64];
	struct small_read {
		uint8_t bytes[13];
	} small_read;

	fd = TEST_SUCC(open_block_device());

	TEST_RES(pread(fd, reference, sizeof(reference), 0),
		 _ret == sizeof(reference));
	TEST_RES(pread(fd, short_read, sizeof(short_read), 0),
		 _ret == sizeof(short_read));
	TEST_RES(memcmp(reference, short_read, sizeof(short_read)), _ret == 0);
	TEST_RES(pread(fd, &small_read, sizeof(small_read), SECTOR_SIZE - 5),
		 _ret == sizeof(small_read));
	TEST_RES(memcmp(reference + SECTOR_SIZE - 5, &small_read,
			sizeof(small_read)),
		 _ret == 0);

	TEST_SUCC(close_block_device(fd));
}
END_TEST()

// Verifies that unaligned block-device writes do not corrupt bytes outside the
// requested write range, including bytes from the neighboring sector.
FN_TEST(unaligned_pwrite_preserves_sector_bytes)
{
	int fd;
	uint8_t original[SECTOR_SIZE * 2];
	uint8_t after[SECTOR_SIZE * 2];
	uint8_t patch[] = { 'X', 'Y', 'Z' };

	fd = TEST_SUCC(open_block_device());

	TEST_RES(pread(fd, original, sizeof(original), 0),
		 _ret == sizeof(original));
	// Write three bytes around the first sector boundary. A buggy
	// read-modify-write path may accidentally change adjacent bytes.
	TEST_RES(pwrite(fd, patch, sizeof(patch), SECTOR_SIZE - 1),
		 _ret == sizeof(patch));
	TEST_RES(pread(fd, after, sizeof(after), 0), _ret == sizeof(after));

	TEST_RES(memcmp(original, after, SECTOR_SIZE - 1), _ret == 0);
	TEST_RES(memcmp(after + SECTOR_SIZE - 1, patch, sizeof(patch)),
		 _ret == 0);
	TEST_RES(memcmp(original + SECTOR_SIZE + 2, after + SECTOR_SIZE + 2,
			sizeof(original) - SECTOR_SIZE - 2),
		 _ret == 0);
	// Leave the block device in the same state for later tests.
	TEST_RES(pwrite(fd, original, sizeof(original), 0),
		 _ret == sizeof(original));

	TEST_SUCC(close_block_device(fd));
}
END_TEST()
