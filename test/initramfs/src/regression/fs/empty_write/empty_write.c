/* SPDX-License-Identifier: MPL-2.0 */

#define _GNU_SOURCE
#include <limits.h>
#include <sys/stat.h>
#include <sys/uio.h>
#include <time.h>
#include <unistd.h>

#include "../../common/test.h"

/* Exercise an offset well beyond the two-byte file. */
#define FAR_PAST_EOF 8192
/* Use 2000-01-01 for atime and mtime to detect unintended updates without sleeping. */
#define OLD_TIMESTAMP_SEC 946684800
/*
 * Keep the initial position distinct from every write offset so the test
 * detects pwrite or pwritev incorrectly moving the descriptor position.
 */
#define POSITIONAL_SEED 7

enum empty_write_op { OP_WRITE, OP_PWRITE, OP_PWRITEV };
static const char *const op_names[] = { "write", "pwrite", "pwritev" };

static char path[PATH_MAX];
static int fd;
static char empty_buffer[1];

static int same_time(struct timespec a, struct timespec b)
{
	return a.tv_sec == b.tv_sec && a.tv_nsec == b.tv_nsec;
}

/* Restores a two-byte file with an old mtime and seeks `fd` to `position`. */
static void reset_fixture(struct stat *before, off_t position)
{
	const struct timespec times[2] = { { OLD_TIMESTAMP_SEC, 0 },
					   { OLD_TIMESTAMP_SEC, 0 } };

	CHECK(ftruncate(fd, 0));
	CHECK_WITH(pwrite(fd, "hi", 2, 0), _ret == 2);
	CHECK(futimens(fd, times));
	CHECK(fstat(fd, before));
	CHECK_WITH(before->st_size, _ret == 2);
	CHECK_WITH(lseek(fd, position, SEEK_SET), _ret == position);
}

static ssize_t empty_write(enum empty_write_op op, off_t offset)
{
	struct iovec iov[2] = {
		{ .iov_base = empty_buffer, .iov_len = 0 },
		{ .iov_base = empty_buffer, .iov_len = 0 },
	};

	switch (op) {
	case OP_WRITE:
		return write(fd, "", 0);
	case OP_PWRITE:
		return pwrite(fd, "", 0, offset);
	case OP_PWRITEV:
		return pwritev(fd, iov, 2, offset);
	}

	errno = EINVAL;
	return -1;
}

FN_SETUP(create_file)
{
	const char *dir = getenv("TEST_TMPDIR");

	if (!dir) {
		dir = "/tmp";
	}
	CHECK_WITH(snprintf(path, sizeof(path), "%s/empty-write-XXXXXX", dir),
		   _ret > 0 && (size_t)_ret < sizeof(path));
	fd = CHECK(mkstemp(path));
	fprintf(stderr, "fixture=%s\n", dir);
}
END_SETUP()

FN_TEST(empty_writes_preserve_metadata_and_position)
{
	const enum empty_write_op ops[] = { OP_WRITE, OP_PWRITE, OP_PWRITEV };
	const off_t offsets[] = { 1, 2, FAR_PAST_EOF };

	for (size_t o = 0; o < sizeof(ops) / sizeof(ops[0]); o++) {
		for (size_t i = 0; i < sizeof(offsets) / sizeof(offsets[0]);
		     i++) {
			off_t position = ops[o] == OP_WRITE ? offsets[i] :
							      POSITIONAL_SEED;
			struct stat before, after;

			fprintf(stderr, "%s offset=%lld\n", op_names[ops[o]],
				(long long)offsets[i]);
			reset_fixture(&before, position);

			TEST_RES(empty_write(ops[o], offsets[i]), _ret == 0);
			CHECK(fstat(fd, &after));
			TEST_RES(after.st_size, _ret == before.st_size);
			TEST_RES(after.st_blocks, _ret == before.st_blocks);
			TEST_RES(same_time(before.st_atim, after.st_atim),
				 _ret);
			TEST_RES(same_time(before.st_mtim, after.st_mtim),
				 _ret);
			TEST_RES(same_time(before.st_ctim, after.st_ctim),
				 _ret);
			/* Empty writes must leave the descriptor position unchanged. */
			TEST_RES(lseek(fd, 0, SEEK_CUR), _ret == position);
		}
	}
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(fd));
	CHECK(unlink(path));
}
END_SETUP()
