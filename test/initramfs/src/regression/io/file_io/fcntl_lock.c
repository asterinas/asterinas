// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <sched.h>
#include <stdint.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

#define TEST_FILE "/tmp/fcntl_lock_regression"
#define CLONE_STACK_SIZE 4096

static char clone_stack[CLONE_STACK_SIZE];

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

static int clone_child_exit(void *arg)
{
	(void)arg;
	return 0;
}

static int clone_child_close(void *arg)
{
	CHECK(close((int)(intptr_t)arg));
	return 0;
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

FN_TEST(process_exit_releases_locks)
{
	int fd = TEST_SUCC(open(TEST_FILE, O_RDWR));

	pid_t child = TEST_SUCC(fork());
	if (child == 0) {
		CHECK(try_write_lock(fd, 0, 100));
		_exit(0);
	}

	/* Exiting the sole owner closes its file table and releases its locks. */
	int status = 0;
	TEST_RES(waitpid(child, &status, 0), _ret == child &&
						     WIFEXITED(status) &&
						     WEXITSTATUS(status) == 0);
	TEST_SUCC(try_write_lock(fd, 0, 100));

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(forked_process_close_keeps_parent_locks)
{
	int fd = TEST_SUCC(open(TEST_FILE, O_RDWR));

	TEST_SUCC(try_write_lock(fd, 0, 100));
	pid_t child = TEST_SUCC(fork());
	if (child == 0) {
		/* fork() gives the child a distinct file table and lock owner. */
		CHECK(close(fd));
		_exit(0);
	}

	int status = 0;
	TEST_RES(waitpid(child, &status, 0), _ret == child &&
						     WIFEXITED(status) &&
						     WEXITSTATUS(status) == 0);
	TEST_RES(child_try_write_lock(0, 100), _ret == EAGAIN);

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(shared_file_table_exit_keeps_locks)
{
	int fd = TEST_SUCC(open(TEST_FILE, O_RDWR));

	TEST_SUCC(try_write_lock(fd, 0, 100));
	/* CLONE_FILES makes the child share the parent's file table. */
	pid_t child = TEST_SUCC(clone(clone_child_exit,
				      clone_stack + sizeof(clone_stack),
				      CLONE_FILES | SIGCHLD, NULL));

	/* One sharer exiting must not release locks owned by the shared table. */
	int status = 0;
	TEST_RES(waitpid(child, &status, 0), _ret == child &&
						     WIFEXITED(status) &&
						     WEXITSTATUS(status) == 0);
	TEST_RES(child_try_write_lock(0, 100), _ret == EAGAIN);

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(shared_file_table_close_releases_locks)
{
	int fd = TEST_SUCC(open(TEST_FILE, O_RDWR));
	int duplicated_fd = TEST_SUCC(dup(fd));

	TEST_SUCC(try_write_lock(fd, 0, 100));
	/* The child closes the duplicate in the file table shared with its parent. */
	pid_t child = TEST_SUCC(
		clone(clone_child_close, clone_stack + sizeof(clone_stack),
		      CLONE_FILES | SIGCHLD, (void *)(intptr_t)duplicated_fd));

	/* Closing any fd for the file releases this table owner's POSIX locks. */
	int status = 0;
	TEST_RES(waitpid(child, &status, 0), _ret == child &&
						     WIFEXITED(status) &&
						     WEXITSTATUS(status) == 0);
	TEST_RES(child_try_write_lock(0, 100), _ret == 0);

	TEST_SUCC(close(fd));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(unlink(TEST_FILE));
}
END_SETUP()
