/* SPDX-License-Identifier: MPL-2.0 */

#include <fcntl.h>
#include <stdio.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../../common/test.h"

#define FILE_NAME "aster_read_eof"
#define CONTENT "hi"
#define CONTENT_LEN (sizeof(CONTENT) - 1)
#define BUF_SIZE 64
#define SENTINEL 0xAA

/* Mount points of the file systems whose reads go through the page cache. */
static const char *const DIRS[] = { "/tmp", "/ext2", "/exfat" };
#define NR_DIRS (sizeof(DIRS) / sizeof(DIRS[0]))

static int fds[NR_DIRS];

/* Returns 1 if `buf[from..BUF_SIZE)` still holds the sentinel. */
static int tail_untouched(const unsigned char *buf, size_t from)
{
	for (size_t i = from; i < BUF_SIZE; i++) {
		if (buf[i] != SENTINEL) {
			return 0;
		}
	}

	return 1;
}

FN_SETUP(create_files)
{
	char path[64];
	struct stat st;

	for (size_t i = 0; i < NR_DIRS; i++) {
		if (stat(DIRS[i], &st) < 0) {
			fds[i] = -1;
			continue;
		}

		CHECK(snprintf(path, sizeof(path), "%s/" FILE_NAME, DIRS[i]));
		unlink(path);

		fds[i] = CHECK(open(path, O_RDWR | O_CREAT | O_TRUNC, 0600));
		CHECK_WITH(write(fds[i], CONTENT, CONTENT_LEN),
			   _ret == CONTENT_LEN);
	}
}
END_SETUP()

FN_TEST(pread_across_eof_leaves_tail_untouched)
{
	unsigned char buf[BUF_SIZE];

	for (size_t i = 0; i < NR_DIRS; i++) {
		if (fds[i] < 0) {
			continue;
		}
		fprintf(stderr, "%s: on %s\n", __func__, DIRS[i]);

		memset(buf, SENTINEL, sizeof(buf));
		TEST_RES(pread(fds[i], buf, sizeof(buf), 1),
			 _ret == CONTENT_LEN - 1);
		TEST_RES(tail_untouched(buf, CONTENT_LEN - 1), _ret == 1);
	}
}
END_TEST()

FN_TEST(pread_at_eof_leaves_buffer_untouched)
{
	unsigned char buf[BUF_SIZE];

	for (size_t i = 0; i < NR_DIRS; i++) {
		if (fds[i] < 0) {
			continue;
		}
		fprintf(stderr, "%s: on %s\n", __func__, DIRS[i]);

		memset(buf, SENTINEL, sizeof(buf));
		TEST_RES(pread(fds[i], buf, sizeof(buf), CONTENT_LEN),
			 _ret == 0);
		TEST_RES(tail_untouched(buf, 0), _ret == 1);
	}
}
END_TEST()

FN_TEST(read_across_eof_leaves_tail_untouched)
{
	unsigned char buf[BUF_SIZE];

	for (size_t i = 0; i < NR_DIRS; i++) {
		if (fds[i] < 0) {
			continue;
		}
		fprintf(stderr, "%s: on %s\n", __func__, DIRS[i]);

		memset(buf, SENTINEL, sizeof(buf));
		TEST_SUCC(lseek(fds[i], 1, SEEK_SET));
		TEST_RES(read(fds[i], buf, sizeof(buf)),
			 _ret == CONTENT_LEN - 1);
		TEST_RES(tail_untouched(buf, CONTENT_LEN - 1), _ret == 1);
	}
}
END_TEST()

FN_SETUP(cleanup)
{
	char path[64];

	for (size_t i = 0; i < NR_DIRS; i++) {
		if (fds[i] < 0) {
			continue;
		}

		CHECK(close(fds[i]));
		CHECK(snprintf(path, sizeof(path), "%s/" FILE_NAME, DIRS[i]));
		CHECK(unlink(path));
	}
}
END_SETUP()
