// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../test.h"

#include <fcntl.h>
#include <sched.h>
#include <sys/wait.h>
#include <unistd.h>

static char child_stack[4096];
#define CHILD_STACK_TOP \
	(child_stack + sizeof(child_stack) / sizeof(child_stack[0]))

static int fd1, fd2;

static int child_close(void *arg)
{
	CHECK(close(fd2));

	_exit(0);
}

FN_TEST(clone_files_and_close)
{
	pid_t pid;
	int status;

	fd1 = TEST_SUCC(open("/dev/null", O_RDONLY));
	fd2 = TEST_SUCC(open("/dev/null", O_RDONLY));

	pid = TEST_SUCC(clone(&child_close, CHILD_STACK_TOP,
			      CLONE_FILES | SIGCHLD, NULL, NULL, NULL, NULL));

	TEST_RES(wait(&status),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);

	TEST_SUCC(close(fd1));
	TEST_ERRNO(close(fd2), EBADF);
}
END_TEST()
