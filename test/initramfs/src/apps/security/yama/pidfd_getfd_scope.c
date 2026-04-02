// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#ifndef SYS_pidfd_getfd
#ifdef __NR_pidfd_getfd
#define SYS_pidfd_getfd __NR_pidfd_getfd
#elif defined(__x86_64__)
#define SYS_pidfd_getfd 438
#endif
#endif

#include "../../common/test.h"

enum {
	YAMA_SCOPE_DISABLED = 0,
	YAMA_SCOPE_RELATIONAL = 1,
	TARGET_FD = 100,
};

static const char *YAMA_PTRACE_SCOPE = "/proc/sys/kernel/yama/ptrace_scope";
static const char *TESTFILE = "/tmp/yama_pidfd_getfd_testfile";
static const char *TEST_CONTENT = "Test content\n";

static int saved_scope = YAMA_SCOPE_RELATIONAL;

static int pidfd_open_syscall(pid_t pid)
{
	return syscall(SYS_pidfd_open, pid, 0);
}

static int pidfd_getfd_syscall(int pidfd, int targetfd)
{
	return syscall(SYS_pidfd_getfd, pidfd, targetfd, 0);
}

static int read_scope(void)
{
	char buf[16] = { 0 };

	int fd = open(YAMA_PTRACE_SCOPE, O_RDONLY);
	if (fd < 0) {
		return -1;
	}

	ssize_t len = read(fd, buf, sizeof(buf) - 1);
	int saved_errno = errno;
	close(fd);
	errno = saved_errno;
	if (len < 0) {
		return -1;
	}

	buf[len] = '\0';
	return atoi(buf);
}

static int write_scope(int scope)
{
	char buf[16];

	int fd = open(YAMA_PTRACE_SCOPE, O_WRONLY);
	if (fd < 0) {
		return -1;
	}

	int len = snprintf(buf, sizeof(buf), "%d\n", scope);
	ssize_t written = write(fd, buf, len);
	int saved_errno = errno;
	close(fd);
	errno = saved_errno;

	if (written != len) {
		if (written >= 0) {
			errno = EIO;
		}
		return -1;
	}

	return 0;
}

static pid_t spawn_target_process(void)
{
	int ready_pipe[2];
	CHECK(pipe(ready_pipe));

	pid_t target_pid = CHECK(fork());
	if (target_pid == 0) {
		close(ready_pipe[0]);

		int fd = CHECK(open(TESTFILE, O_CREAT | O_RDWR | O_TRUNC, 0644));
		CHECK_WITH(write(fd, TEST_CONTENT, strlen(TEST_CONTENT)),
			   _ret == (ssize_t)strlen(TEST_CONTENT));
		CHECK(dup2(fd, TARGET_FD));
		CHECK(close(fd));
		CHECK_WITH(write(ready_pipe[1], "1", 1), _ret == 1);
		CHECK(close(ready_pipe[1]));

		pause();
		_exit(EXIT_SUCCESS);
	}

	close(ready_pipe[1]);
	char ready = '\0';
	CHECK_WITH(read(ready_pipe[0], &ready, 1), _ret == 1);
	CHECK(close(ready_pipe[0]));

	return target_pid;
}

static int run_sibling_pidfd_getfd(pid_t target_pid, bool expect_success)
{
	pid_t attacker_pid = fork();
	if (attacker_pid < 0) {
		return -1;
	}

	if (attacker_pid == 0) {
		int pidfd = pidfd_open_syscall(target_pid);
		if (pidfd < 0) {
			_exit(10);
		}

		int duplicated_fd = pidfd_getfd_syscall(pidfd, TARGET_FD);
		if (!expect_success) {
			int saved_errno = errno;
			close(pidfd);
			if (duplicated_fd >= 0) {
				close(duplicated_fd);
				_exit(11);
			}
			if (saved_errno != EPERM) {
				_exit(12);
			}
			_exit(EXIT_SUCCESS);
		}

		if (duplicated_fd < 0) {
			close(pidfd);
			_exit(20);
		}

		char buf[32] = { 0 };
		ssize_t len = pread(duplicated_fd, buf, sizeof(buf) - 1, 0);
		close(duplicated_fd);
		close(pidfd);
		if (len < 0) {
			_exit(21);
		}

		buf[len] = '\0';
		if (strcmp(buf, TEST_CONTENT) != 0) {
			_exit(22);
		}

		_exit(EXIT_SUCCESS);
	}

	int status = 0;
	if (waitpid(attacker_pid, &status, 0) < 0) {
		return -1;
	}

	if (!WIFEXITED(status) || WEXITSTATUS(status) != 0) {
		errno = ECHILD;
		return -1;
	}

	return 0;
}

static void cleanup_target_process(pid_t target_pid)
{
	if (target_pid <= 0) {
		return;
	}

	kill(target_pid, SIGKILL);
	waitpid(target_pid, NULL, 0);
	unlink(TESTFILE);
}

FN_SETUP(save_initial_scope)
{
	saved_scope = CHECK(read_scope());
}
END_SETUP()

FN_TEST(yama_procfs_roundtrip)
{
	TEST_SUCC(write_scope(YAMA_SCOPE_DISABLED));
	TEST_RES(read_scope(), _ret == YAMA_SCOPE_DISABLED);

	TEST_SUCC(write_scope(saved_scope));
	TEST_RES(read_scope(), _ret == saved_scope);
}
END_TEST()

FN_TEST(yama_relational_denies_sibling_pidfd_getfd)
{
	TEST_SUCC(write_scope(YAMA_SCOPE_RELATIONAL));
	pid_t target_pid = CHECK(spawn_target_process());

	TEST_SUCC(run_sibling_pidfd_getfd(target_pid, false));

	cleanup_target_process(target_pid);
	TEST_SUCC(write_scope(saved_scope));
}
END_TEST()

FN_TEST(yama_disabled_allows_sibling_pidfd_getfd)
{
	TEST_SUCC(write_scope(YAMA_SCOPE_DISABLED));
	pid_t target_pid = CHECK(spawn_target_process());

	TEST_SUCC(run_sibling_pidfd_getfd(target_pid, true));

	cleanup_target_process(target_pid);
	TEST_SUCC(write_scope(saved_scope));
}
END_TEST()
