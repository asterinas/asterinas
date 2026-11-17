// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <linux/falloc.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"

#define BASE_DIR "/ext2/fallocate"
#define BLOCK_SIZE 4096

static char zero_buf[BLOCK_SIZE];
static char rbuf[BLOCK_SIZE] __attribute__((aligned(BLOCK_SIZE)));
static char wbuf[BLOCK_SIZE] __attribute__((aligned(BLOCK_SIZE)));

FN_SETUP(prepare_base_dir)
{
	CHECK_WITH(mkdir(BASE_DIR, 0755), _ret == 0 || errno == EEXIST);
}
END_SETUP()

FN_TEST(fallocate_extends_size)
{
	const char *path = BASE_DIR "/extends_size";
	struct stat st;

	int fd = TEST_SUCC(open(path, O_CREAT | O_RDWR, 0644));
	TEST_SUCC(fallocate(fd, 0, 0, 4096));
	TEST_SUCC(fstat(fd, &st));
	TEST_RES(fstat(fd, &st), st.st_size == 4096);
	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(path));
}
END_TEST()

FN_TEST(fallocate_o_direct)
{
	const char *path = BASE_DIR "/o_direct";
	struct stat st;

	// `O_DIRECT` does not restrict `fallocate`.
	int fd = TEST_SUCC(open(path, O_CREAT | O_RDWR | O_DIRECT, 0644));
	TEST_SUCC(fallocate(fd, 0, 0, BLOCK_SIZE * 2));
	TEST_RES(fstat(fd, &st), st.st_size == BLOCK_SIZE * 2);

	// The allocated range reads as zeros.
	memset(rbuf, 0xff, BLOCK_SIZE);
	TEST_RES(pread(fd, rbuf, BLOCK_SIZE, 0), _ret == BLOCK_SIZE);
	TEST_RES(memcmp(rbuf, zero_buf, BLOCK_SIZE), _ret == 0);

	// Data written into the allocated range reads back, with and without
	// `O_DIRECT`, and the size does not change.
	memset(wbuf, 0xab, BLOCK_SIZE);
	TEST_RES(pwrite(fd, wbuf, BLOCK_SIZE, BLOCK_SIZE), _ret == BLOCK_SIZE);
	TEST_RES(fstat(fd, &st), st.st_size == BLOCK_SIZE * 2);
	memset(rbuf, 0, BLOCK_SIZE);
	TEST_RES(pread(fd, rbuf, BLOCK_SIZE, BLOCK_SIZE), _ret == BLOCK_SIZE);
	TEST_RES(memcmp(rbuf, wbuf, BLOCK_SIZE), _ret == 0);

	int buffered_fd = TEST_SUCC(open(path, O_RDONLY));
	memset(rbuf, 0, BLOCK_SIZE);
	TEST_RES(pread(buffered_fd, rbuf, BLOCK_SIZE, BLOCK_SIZE),
		 _ret == BLOCK_SIZE);
	TEST_RES(memcmp(rbuf, wbuf, BLOCK_SIZE), _ret == 0);
	TEST_SUCC(close(buffered_fd));

	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(path));
}
END_TEST()

FN_TEST(fallocate_bad_mode)
{
	const char *path = BASE_DIR "/bad_mode";

	int fd = TEST_SUCC(open(path, O_CREAT | O_RDWR, 0644));
	TEST_ERRNO(fallocate(fd, FALLOC_FL_COLLAPSE_RANGE, 0, 4096),
		   EOPNOTSUPP);
	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(path));
}
END_TEST()

FN_TEST(fallocate_keep_size)
{
	const char *path = BASE_DIR "/keep_size";
	struct stat st;

	int fd = TEST_SUCC(open(path, O_CREAT | O_RDWR, 0644));
	TEST_SUCC(fallocate(fd, FALLOC_FL_KEEP_SIZE, 0, 8192));
	TEST_SUCC(fstat(fd, &st));
	TEST_RES(fstat(fd, &st), st.st_size == 0);
	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(path));
}
END_TEST()

FN_TEST(fallocate_keep_size_within)
{
	const char *path = BASE_DIR "/keep_within";
	struct stat st;

	int fd = TEST_SUCC(open(path, O_CREAT | O_RDWR, 0644));
	TEST_RES(write(fd, "hello", 5), _ret == 5);
	TEST_SUCC(fallocate(fd, FALLOC_FL_KEEP_SIZE, 0, 4096));
	TEST_SUCC(fstat(fd, &st));
	TEST_RES(fstat(fd, &st), st.st_size == 5);
	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(path));
}
END_TEST()
