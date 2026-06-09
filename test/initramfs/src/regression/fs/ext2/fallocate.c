// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <linux/falloc.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"

#define BASE_DIR "/ext2/fallocate"

#ifndef FALLOC_FL_KEEP_SIZE
#define FALLOC_FL_KEEP_SIZE 0x01
#endif

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
