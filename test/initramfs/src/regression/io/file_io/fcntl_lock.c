// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define TEST_FILE "/tmp/fcntl_lock_regression"

static int open_test_file(void)
{
	return open(TEST_FILE, O_CREAT | O_RDWR | O_TRUNC, 0666);
}

static int try_write_lock(int fd, off_t start, off_t len)
{
	struct flock lock = {
		.l_type = F_WRLCK,
		.l_whence = SEEK_SET,
		.l_start = start,
		.l_len = len,
	};

	return fcntl(fd, F_SETLK, &lock);
}

static int unlock_range(int fd, off_t start, off_t len)
{
	struct flock lock = {
		.l_type = F_UNLCK,
		.l_whence = SEEK_SET,
		.l_start = start,
		.l_len = len,
	};

	return fcntl(fd, F_SETLK, &lock);
}

static int child_try_write_lock(off_t start, off_t len)
{
	pid_t child = CHECK(fork());
	if (child == 0) {
		int fd = CHECK(open(TEST_FILE, O_RDWR));
		int ret = try_write_lock(fd, start, len);

		if (ret == 0) {
			_exit(0);
		}

		_exit(errno);
	}

	int status = 0;
	CHECK(waitpid(child, &status, 0));
	if (!WIFEXITED(status)) {
		errno = ECHILD;
		return -1;
	}

	return WEXITSTATUS(status);
}

FN_SETUP(create)
{
	int fd = CHECK(open_test_file());
	CHECK(close(fd));
}
END_SETUP()

FN_TEST(unlock_middle_range)
{
	int fd = TEST_SUCC(open(TEST_FILE, O_RDWR));

	TEST_SUCC(try_write_lock(fd, 0, 100));
	TEST_SUCC(unlock_range(fd, 20, 60));
	TEST_RES(child_try_write_lock(20, 60), _ret == 0);

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(close_dup_fd_releases_locks)
{
	int fd = TEST_SUCC(open(TEST_FILE, O_RDWR));
	int duplicated_fd = TEST_SUCC(dup(fd));

	TEST_SUCC(try_write_lock(fd, 0, 100));
	TEST_SUCC(close(duplicated_fd));
	TEST_RES(child_try_write_lock(0, 100), _ret == 0);

	TEST_SUCC(close(fd));
}
END_TEST()
