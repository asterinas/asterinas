// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <sched.h>
#include <signal.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/syscall.h>
#include <sys/wait.h>

#include "../../common/test.h"

FN_TEST(set_ns_empty_flags)
{
	int fd_ns = TEST_SUCC(open("/proc/self/ns/user", O_RDONLY));
	TEST_ERRNO(setns(fd_ns, 0), EINVAL);
	TEST_SUCC(close(fd_ns));

	pid_t pid = getpid();
	int pidfd = TEST_SUCC(syscall(SYS_pidfd_open, pid, 0));
	TEST_ERRNO(setns(pidfd, 0), EINVAL);
	TEST_SUCC(close(pidfd));
}
END_TEST()

FN_TEST(set_self_ns)
{
	// It is not permitted to use setns() to reenter the caller's
	// current user namespace. This is different from other namespaces.
	int fd_ns = TEST_SUCC(open("/proc/self/ns/user", O_RDONLY));
	TEST_ERRNO(setns(fd_ns, CLONE_NEWUSER), EINVAL);
	TEST_SUCC(close(fd_ns));

	pid_t pid = getpid();
	int pidfd = TEST_SUCC(syscall(SYS_pidfd_open, pid, 0));
	TEST_ERRNO(setns(pidfd, CLONE_NEWUSER), EINVAL);
	TEST_SUCC(close(pidfd));
}
END_TEST()

FN_TEST(set_pidfd_self_uts)
{
	pid_t pid = getpid();
	int pidfd = TEST_SUCC(syscall(SYS_pidfd_open, pid, 0));

	TEST_SUCC(setns(pidfd, CLONE_NEWUTS));

	TEST_SUCC(close(pidfd));
}
END_TEST()

FN_TEST(set_pidfd_reaped_process)
{
	pid_t pid = TEST_SUCC(fork());
	if (pid == 0)
		_exit(0);

	int pidfd = TEST_SUCC(syscall(SYS_pidfd_open, pid, 0));

	int status;
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);

	TEST_ERRNO(setns(pidfd, CLONE_NEWUTS), ESRCH);

	TEST_SUCC(close(pidfd));
}
END_TEST()

FN_TEST(set_pidfd_exited_process)
{
	int pipefd[2];
	TEST_SUCC(pipe(pipefd));

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(close(pipefd[0]));
		char ok = 'K';
		CHECK_WITH(write(pipefd[1], &ok, 1), _ret == 1);
		CHECK(close(pipefd[1]));
		pause();
		_exit(1);
	}

	TEST_SUCC(close(pipefd[1]));
	char buf;
	TEST_RES(read(pipefd[0], &buf, 1), _ret == 1 && buf == 'K');
	TEST_SUCC(close(pipefd[0]));

	int pidfd = TEST_SUCC(syscall(SYS_pidfd_open, pid, 0));
	TEST_SUCC(kill(pid, SIGKILL));
	TEST_SUCC(waitid(P_PID, pid, NULL, WNOWAIT | WEXITED));

	TEST_ERRNO(setns(pidfd, CLONE_NEWUTS), ESRCH);

	int status;
	TEST_RES(waitpid(pid, &status, 0), _ret == pid && WIFSIGNALED(status) &&
						   WTERMSIG(status) == SIGKILL);

	TEST_SUCC(close(pidfd));
}
END_TEST()

FN_TEST(set_ns_file_flags_mismatch)
{
	int fd_ns = TEST_SUCC(open("/proc/self/ns/uts", O_RDONLY));

	TEST_ERRNO(setns(fd_ns, CLONE_NEWIPC), EINVAL);
	TEST_ERRNO(setns(fd_ns, CLONE_NEWNET), EINVAL);

	TEST_SUCC(close(fd_ns));
}
END_TEST()

FN_TEST(set_ns_non_namespace_fd)
{
	int pipefd[2];
	TEST_SUCC(pipe(pipefd));

	TEST_ERRNO(setns(pipefd[0], 0), EINVAL);
	TEST_ERRNO(setns(pipefd[0], CLONE_NEWUTS), EINVAL);

	TEST_SUCC(close(pipefd[0]));
	TEST_SUCC(close(pipefd[1]));
}
END_TEST()
