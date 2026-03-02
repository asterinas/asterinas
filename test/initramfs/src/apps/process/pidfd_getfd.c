// SPDX-License-Identifier: MPL-2.0

#include <unistd.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <pthread.h>
#include <string.h>
#include <stdio.h>
#include <stdlib.h>
#include <errno.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/wait.h>
#include "../common/test.h"

static int pidfd;
static pid_t pid;
static int fd;
static int target_fd;
static const char *TESTFILE = "/tmp/pidfd_getfd_testfile";
static int invalid_pidfd = -1;

static int pidfd_open(pid_t pid, unsigned int flags)
{
	return syscall(SYS_pidfd_open, pid, flags);
}

static int pidfd_getfd(int pidfd, int targetfd, unsigned int flags)
{
	return syscall(SYS_pidfd_getfd, pidfd, targetfd, flags);
}

FN_TEST(pidfd_getfd_valid)
{
	size_t size = sizeof(int);
	volatile int *shared_fd =
		TEST_SUCC(mmap(NULL, size, PROT_READ | PROT_WRITE,
			       MAP_SHARED | MAP_ANONYMOUS, -1, 0));

	pid = TEST_SUCC(fork());
	if (pid == 0) {
		fd = CHECK(open(TESTFILE, O_CREAT | O_RDWR | O_TRUNC, 0644));

		CHECK_WITH(write(fd, "Test content\n", 13), _ret == 13);
		*shared_fd = fd;

		pause();
		exit(0);
	}

	while (*shared_fd == 0) {
		usleep(100);
	}
	fd = *shared_fd;

	pidfd = TEST_SUCC(pidfd_open(pid, 0));
	target_fd = TEST_SUCC(pidfd_getfd(pidfd, fd, 0));

	char buffer[128] = { 0 };
	TEST_RES(pread(target_fd, buffer, sizeof(buffer), 0),
		 strcmp(buffer, "Test content\n") == 0);

	TEST_SUCC(munmap((void *)shared_fd, size));
	TEST_SUCC(close(target_fd));
	TEST_SUCC(close(pidfd));
	TEST_SUCC(unlink(TESTFILE));

	TEST_SUCC(kill(pid, SIGKILL));
	TEST_SUCC(waitpid(pid, NULL, 0));
}
END_TEST()

FN_TEST(pidfd_getfd_after_child_exits)
{
	pid = TEST_SUCC(fork());
	if (pid == 0) {
		exit(0);
	}
	TEST_SUCC(waitid(P_PID, pid, NULL,
			 WNOWAIT | WEXITED)); // Ensure the child has exited
	pidfd = TEST_SUCC(pidfd_open(pid, 0));

	TEST_ERRNO(pidfd_getfd(pidfd, fd, 0), ESRCH);
	TEST_SUCC(waitpid(pid, NULL, 0));
	TEST_ERRNO(pidfd_getfd(pidfd, fd, 0), ESRCH);

	TEST_SUCC(close(pidfd));
}
END_TEST()

FN_TEST(pidfd_getfd_errnos)
{
	pid = TEST_SUCC(fork());
	if (pid == 0) {
		exit(0);
	}
	TEST_SUCC(waitid(P_PID, pid, NULL,
			 WNOWAIT | WEXITED)); // Ensure the child has exited
	pidfd = TEST_SUCC(pidfd_open(pid, 0));

	TEST_ERRNO(pidfd_getfd(invalid_pidfd, fd, 0), EBADF);
	TEST_ERRNO(pidfd_getfd(pidfd, -1, 0), ESRCH);
	TEST_ERRNO(pidfd_getfd(pidfd, fd, 1), EINVAL);

	TEST_SUCC(waitpid(pid, NULL, 0));
	TEST_SUCC(close(pidfd));
}
END_TEST()