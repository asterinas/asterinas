// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <fcntl.h>
#include <unistd.h>

#include "../../common/test.h"

#define TEST_FILE "/tmp/asterinas_syncfs_test"

FN_TEST(syncfs_fd_semantics)
{
	int fd;
	int pipe_fds[2];

	unlink(TEST_FILE);

	fd = TEST_SUCC(open(TEST_FILE, O_CREAT | O_RDWR | O_TRUNC, 0600));
	TEST_SUCC(syncfs(fd));
	TEST_SUCC(close(fd));
	TEST_SUCC(unlink(TEST_FILE));

	fd = TEST_SUCC(open("/", O_RDONLY | O_DIRECTORY));
	TEST_SUCC(syncfs(fd));
	TEST_SUCC(close(fd));

	TEST_SUCC(pipe(pipe_fds));
	TEST_SUCC(syncfs(pipe_fds[0]));
	TEST_SUCC(syncfs(pipe_fds[1]));
	TEST_SUCC(close(pipe_fds[0]));
	TEST_SUCC(close(pipe_fds[1]));

	fd = TEST_SUCC(open("/", O_PATH | O_DIRECTORY));
	TEST_ERRNO(syncfs(fd), EBADF);
	TEST_SUCC(close(fd));

	TEST_ERRNO(syncfs(-1), EBADF);
}
END_TEST()
