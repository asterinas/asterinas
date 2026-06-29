// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <unistd.h>

#include "../../common/test.h"

#define TEST_FILE "/tmp/syncfs_test_file"

FN_TEST(syncfs_file)
{
	int fd = TEST_SUCC(open(TEST_FILE, O_CREAT | O_RDWR | O_TRUNC, 0600));

	TEST_SUCC(syncfs(fd));
	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(TEST_FILE));
}
END_TEST()

FN_TEST(syncfs_pipe)
{
	int fds[2];

	TEST_SUCC(pipe(fds));
	TEST_SUCC(syncfs(fds[0]));
	TEST_SUCC(syncfs(fds[1]));
	TEST_SUCC(close(fds[0]));
	TEST_SUCC(close(fds[1]));
}
END_TEST()

FN_TEST(syncfs_bad_fd)
{
	TEST_ERRNO(syncfs(-1), EBADF);
}
END_TEST()

FN_TEST(syncfs_opath_fd)
{
	int fd = TEST_SUCC(open(TEST_FILE, O_CREAT | O_RDWR | O_TRUNC, 0600));
	TEST_SUCC(close(fd));

	fd = TEST_SUCC(open(TEST_FILE, O_PATH));
	TEST_ERRNO(syncfs(fd), EBADF);
	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(TEST_FILE));
}
END_TEST()
