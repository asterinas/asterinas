// SPDX-License-Identifier: MPL-2.0

#include "../test.h"

#include <unistd.h>
#include <sys/wait.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/poll.h>
#include <fcntl.h>
#include <signal.h>

static int child_pid;
static int pid_fd;

FN_SETUP(create_child)
{
	child_pid = CHECK(fork());

	if (child_pid == 0) {
		while (1) {
			usleep(100);
		}
		exit(EXIT_SUCCESS);
	}
}
END_SETUP()

FN_TEST(pidfd_open)
{
	pid_fd = TEST_SUCC(syscall(SYS_pidfd_open, child_pid, 0));
}
END_TEST()

FN_TEST(read_write)
{
	char buf[1] = {};
	TEST_ERRNO(read(pid_fd, buf, 1), EINVAL);
	TEST_ERRNO(pread(pid_fd, buf, 1, 0), ESPIPE);
	TEST_ERRNO(write(pid_fd, "a", 1), EINVAL);
	TEST_ERRNO(pwrite(pid_fd, "b", 1, 0), ESPIPE);
}
END_TEST()

FN_TEST(set_nonblocking)
{
	int flags =
		TEST_RES(fcntl(pid_fd, F_GETFL, 0), (_ret & O_NONBLOCK) == 0);
	TEST_SUCC(fcntl(pid_fd, F_SETFL, flags | O_NONBLOCK));
	TEST_RES(fcntl(pid_fd, F_GETFL, 0), (_ret & O_NONBLOCK) != 0);
}
END_TEST()

FN_TEST(file_stat)
{
	struct stat file_info;
	TEST_RES(fstat(pid_fd, &file_info),
		 file_info.st_mode == 0600 && file_info.st_size == 0 &&
			 file_info.st_blksize == 4096);
}
END_TEST()

#define POLL_EVENTS (POLLIN | POLLOUT | POLLHUP | POLLERR)
static struct pollfd pfd;

FN_TEST(poll)
{
	pfd.fd = pid_fd;
	pfd.events = POLL_EVENTS;
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == 0);

	TEST_SUCC(kill(child_pid, SIGKILL));
	sleep(1);
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == POLLIN);
}
END_TEST()

FN_TEST(wait)
{
#define P_PIDFD 3
	TEST_SUCC(waitid(P_PIDFD, pid_fd, NULL, WNOHANG | WEXITED));
	pfd.revents = 0;
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == POLLIN);
	TEST_ERRNO(waitid(P_PIDFD, pid_fd, NULL, WNOHANG | WEXITED), ECHILD);
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(pid_fd));
}
END_SETUP()