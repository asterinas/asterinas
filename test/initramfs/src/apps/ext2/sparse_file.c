// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <linux/fs.h>
#include "../test.h"

#define TEST_FILE_PATH "/ext2/test_sparse_file"
#define TEST_BLOCK_SIZE 4096

static int check_all_pattern(const void *buf, size_t len, unsigned char pattern)
{
	const unsigned char *p = buf;
	for (size_t i = 0; i < len; i++) {
		if (p[i] != pattern) {
			return 0;
		}
	}
	return 1;
}

static int check_all_zeros(const void *buf, size_t len)
{
	return check_all_pattern(buf, len, 0);
}

// Create sparse blocks by lseek beyond EOF and read should return zeros
FN_TEST(lseek_create_sparse_blocks)
{
	int fd = TEST_SUCC(
		open(TEST_FILE_PATH, O_RDWR | O_CREAT | O_TRUNC, 0644));

	// Write some data at the beginning
	char write_buf[128] = "Hello, sparse file!";
	TEST_RES(write(fd, write_buf, sizeof(write_buf)),
		 _ret == sizeof(write_buf));

	// Seek far beyond the current file size to create sparse blocks
	off_t new_offset = 16 * 1024; // 16KB offset
	off_t result = TEST_SUCC(lseek(fd, new_offset, SEEK_SET));
	TEST_RES(result, _ret == new_offset);

	// Write more data at the new position - this creates a sparse file
	char write_buf2[64] = "Data after sparse area";
	TEST_RES(write(fd, write_buf2, sizeof(write_buf2)),
		 _ret == sizeof(write_buf2));

	// Get the file size
	off_t file_size = TEST_SUCC(lseek(fd, 0, SEEK_END));
	TEST_RES(file_size, _ret == new_offset + sizeof(write_buf2));

	// Read from the beginning - should get original data
	TEST_SUCC(lseek(fd, 0, SEEK_SET));
	char read_buf[128] = { 0 };
	TEST_RES(read(fd, read_buf, sizeof(read_buf)),
		 _ret == sizeof(read_buf) &&
			 memcmp(read_buf, write_buf, sizeof(write_buf)) == 0);

	// Read from the sparse area - should get all zeros
	TEST_SUCC(lseek(fd, 512, SEEK_SET)); // Position in the sparse area
	char sparse_buf[512] = { 0 };
	TEST_RES(read(fd, sparse_buf, sizeof(sparse_buf)),
		 _ret == sizeof(sparse_buf) &&
			 check_all_zeros(sparse_buf, sizeof(sparse_buf)));

	// Read the data after the sparse area
	TEST_SUCC(lseek(fd, new_offset, SEEK_SET));
	char read_buf2[64] = { 0 };
	TEST_RES(read(fd, read_buf2, sizeof(read_buf2)),
		 _ret == sizeof(read_buf2) && memcmp(read_buf2, write_buf2,
						     sizeof(write_buf2)) == 0);

	// Clean up
	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(TEST_FILE_PATH));
}
END_TEST()

// Punch hole with fallocate
FN_TEST(fallocate_punch_hole)
{
	int fd = TEST_SUCC(
		open(TEST_FILE_PATH, O_RDWR | O_CREAT | O_TRUNC, 0644));

	// Write data to multiple blocks
	char data[TEST_BLOCK_SIZE * 4];
	memset(data, 'A', sizeof(data));
	TEST_RES(write(fd, data, sizeof(data)), _ret == sizeof(data));

	// Sync to ensure data is written
	TEST_SUCC(fsync(fd));

	// Punch a hole in the middle two blocks (offset 4096, length 8192)
	off_t punch_offset = TEST_BLOCK_SIZE;
	off_t punch_len = TEST_BLOCK_SIZE * 2;
	TEST_SUCC(fallocate(fd, FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE,
			    punch_offset, punch_len));

	// File size should remain the same
	off_t file_size = TEST_SUCC(lseek(fd, 0, SEEK_END));
	TEST_RES(file_size, _ret == sizeof(data));

	// Read from the punched hole area - should get zeros
	TEST_SUCC(lseek(fd, punch_offset, SEEK_SET));
	char hole_buf[punch_len];
	TEST_RES(read(fd, hole_buf, sizeof(hole_buf)),
		 _ret == sizeof(hole_buf) &&
			 check_all_zeros(hole_buf, sizeof(hole_buf)));

	// First block should still have data
	TEST_SUCC(lseek(fd, 0, SEEK_SET));
	char first_block[TEST_BLOCK_SIZE];
	TEST_RES(read(fd, first_block, sizeof(first_block)),
		 _ret == sizeof(first_block) &&
			 check_all_pattern(first_block, sizeof(first_block),
					   'A'));

	// Last block should still have data
	TEST_SUCC(lseek(fd, TEST_BLOCK_SIZE * 3, SEEK_SET));
	char last_block[TEST_BLOCK_SIZE];
	TEST_RES(read(fd, last_block, sizeof(last_block)),
		 _ret == sizeof(last_block) &&
			 check_all_pattern(last_block, sizeof(last_block),
					   'A'));

	// Clean up
	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(TEST_FILE_PATH));
}
END_TEST()

// Write to sparse block should allocate and persist data
FN_TEST(write_to_sparse_block)
{
	int fd = TEST_SUCC(
		open(TEST_FILE_PATH, O_RDWR | O_CREAT | O_TRUNC, 0644));

	// Create a sparse file by seeking beyond EOF
	off_t sparse_offset = 32 * 1024; // 32KB
	TEST_SUCC(lseek(fd, sparse_offset, SEEK_SET));

	// Write data to the sparse area
	char write_data[512] = "Data written to sparse block";
	TEST_RES(write(fd, write_data, sizeof(write_data)),
		 _ret == sizeof(write_data));

	// Sync to ensure data is written
	TEST_SUCC(fsync(fd));

	// Read back the written data
	TEST_SUCC(lseek(fd, sparse_offset, SEEK_SET));
	char read_data[512] = { 0 };
	TEST_RES(read(fd, read_data, sizeof(read_data)),
		 _ret == sizeof(read_data) && memcmp(read_data, write_data,
						     sizeof(write_data)) == 0);

	// Read from the sparse area before the written data - should be zeros
	TEST_SUCC(lseek(fd, TEST_BLOCK_SIZE, SEEK_SET));
	char sparse_buf[512] = { 0 };
	TEST_RES(read(fd, sparse_buf, sizeof(sparse_buf)),
		 _ret == sizeof(sparse_buf) &&
			 check_all_zeros(sparse_buf, sizeof(sparse_buf)));

	// Close and reopen to verify persistence
	TEST_SUCC(close(fd));
	fd = TEST_SUCC(open(TEST_FILE_PATH, O_RDONLY, 0644));

	// Verify the data is still there after reopening
	TEST_SUCC(lseek(fd, sparse_offset, SEEK_SET));
	memset(read_data, 0, sizeof(read_data));
	TEST_RES(read(fd, read_data, sizeof(read_data)),
		 _ret == sizeof(read_data) && memcmp(read_data, write_data,
						     sizeof(write_data)) == 0);

	// Clean up
	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(TEST_FILE_PATH));
}
END_TEST()

// Truncate sparse file
FN_TEST(truncate_sparse_file)
{
	int fd = TEST_SUCC(
		open(TEST_FILE_PATH, O_RDWR | O_CREAT | O_TRUNC, 0644));

	// Create a sparse file
	TEST_SUCC(lseek(fd, 64 * 1024, SEEK_SET)); // 64KB
	char data[128] = "End of sparse file";
	TEST_RES(write(fd, data, sizeof(data)), _ret == sizeof(data));

	off_t original_size = TEST_SUCC(lseek(fd, 0, SEEK_END));
	TEST_RES(original_size, _ret == 64 * 1024 + sizeof(data));

	// Truncate to smaller size (should cut off the end)
	off_t new_size = 32 * 1024;
	TEST_SUCC(ftruncate(fd, new_size));

	off_t truncated_size = TEST_SUCC(lseek(fd, 0, SEEK_END));
	TEST_RES(truncated_size, _ret == new_size);

	// Read from truncated area - should get zeros (sparse)
	TEST_SUCC(lseek(fd, new_size - 512, SEEK_SET));
	char buf[512] = { 0 };
	TEST_RES(read(fd, buf, sizeof(buf)),
		 _ret == sizeof(buf) && check_all_zeros(buf, sizeof(buf)));

	// Truncate to larger size (should extend with zeros)
	new_size = 128 * 1024;
	TEST_SUCC(ftruncate(fd, new_size));

	truncated_size = TEST_SUCC(lseek(fd, 0, SEEK_END));
	TEST_RES(truncated_size, _ret == new_size);

	// Read from newly extended sparse area - should be zeros
	TEST_SUCC(lseek(fd, 100 * 1024, SEEK_SET));
	memset(buf, 0, sizeof(buf));
	TEST_RES(read(fd, buf, sizeof(buf)),
		 _ret == sizeof(buf) && check_all_zeros(buf, sizeof(buf)));

	// Clean up
	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(TEST_FILE_PATH));
}
END_TEST()
