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

#include "../../common/capability.h"

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
	int fd = CHECK(open(YAMA_PTRACE_SCOPE, O_RDONLY));
	ssize_t len = CHECK(read(fd, buf, sizeof(buf) - 1));
	CHECK(close(fd));

	buf[len] = '\0';
	return atoi(buf);
}

static int write_scope(int scope)
{
	char buf[16];
	int fd = CHECK(open(YAMA_PTRACE_SCOPE, O_WRONLY));
	int len = CHECK(snprintf(buf, sizeof(buf), "%d\n", scope));
	CHECK_WITH(write(fd, buf, len), _ret == len);
	return close(fd);
}

static pid_t spawn_target_process(bool drop_ptrace_cap)
{
	int ready_pipe[2];
	CHECK(pipe(ready_pipe));

	pid_t target_pid = CHECK(fork());
	if (target_pid == 0) {
		CHECK(close(ready_pipe[0]));
		if (drop_ptrace_cap) {
			drop_capability(CAP_SYS_PTRACE);
		}

		int fd =
			CHECK(open(TESTFILE, O_CREAT | O_RDWR | O_TRUNC, 0644));
		CHECK_WITH(write(fd, TEST_CONTENT, strlen(TEST_CONTENT)),
			   _ret == (ssize_t)strlen(TEST_CONTENT));
		CHECK(dup2(fd, TARGET_FD));
		CHECK(close(fd));
		CHECK_WITH(write(ready_pipe[1], "1", 1), _ret == 1);
		CHECK(close(ready_pipe[1]));

		pause();
		_exit(EXIT_SUCCESS);
	}

	CHECK(close(ready_pipe[1]));
	char ready = '\0';
	CHECK_WITH(read(ready_pipe[0], &ready, 1), _ret == 1);
	CHECK(close(ready_pipe[0]));
	return target_pid;
}

static void cleanup_target_process(pid_t target_pid)
{
	if (target_pid > 0) {
		kill(target_pid, SIGKILL);
		waitpid(target_pid, NULL, 0);
	}
	unlink(TESTFILE);
}

FN_SETUP(save_initial_scope)
{
	saved_scope = CHECK(read_scope());
}
END_SETUP()

FN_TEST(procfs_roundtrip)
{
	TEST_SUCC(write_scope(YAMA_SCOPE_DISABLED));
	TEST_RES(read_scope(), _ret == YAMA_SCOPE_DISABLED);

	TEST_SUCC(write_scope(saved_scope));
	TEST_RES(read_scope(), _ret == saved_scope);
}
END_TEST()

FN_TEST(relational_denies_sibling_pidfd_getfd)
{
	TEST_SUCC(write_scope(YAMA_SCOPE_RELATIONAL));
	pid_t target_pid = TEST_SUCC(spawn_target_process(true));
	pid_t attacker_pid = TEST_SUCC(fork());

	if (attacker_pid == 0) {
		drop_capability(CAP_SYS_PTRACE);

		int pidfd = CHECK(pidfd_open_syscall(target_pid));

		CHECK_WITH(pidfd_getfd_syscall(pidfd, TARGET_FD),
			   _ret < 0 && errno == EPERM);
		CHECK(close(pidfd));
		_exit(EXIT_SUCCESS);
	}

	int status = 0;
	TEST_RES(waitpid(attacker_pid, &status, 0),
		 WIFEXITED(status) && WEXITSTATUS(status) == 0);

	cleanup_target_process(target_pid);
	TEST_SUCC(write_scope(saved_scope));
}
END_TEST()

FN_TEST(disabled_allows_sibling_pidfd_getfd)
{
	TEST_SUCC(write_scope(YAMA_SCOPE_DISABLED));
	pid_t target_pid = TEST_SUCC(spawn_target_process(true));
	pid_t attacker_pid = TEST_SUCC(fork());

	if (attacker_pid == 0) {
		drop_capability(CAP_SYS_PTRACE);

		int pidfd = CHECK(pidfd_open_syscall(target_pid));
		int duplicated_fd =
			CHECK(pidfd_getfd_syscall(pidfd, TARGET_FD));

		char buf[32] = { 0 };
		ssize_t len =
			CHECK(pread(duplicated_fd, buf, sizeof(buf) - 1, 0));
		CHECK(close(duplicated_fd));
		CHECK(close(pidfd));

		buf[len] = '\0';
		CHECK_WITH(strcmp(buf, TEST_CONTENT), _ret == 0);

		_exit(EXIT_SUCCESS);
	}

	int status = 0;
	TEST_RES(waitpid(attacker_pid, &status, 0),
		 WIFEXITED(status) && WEXITSTATUS(status) == 0);

	cleanup_target_process(target_pid);
	TEST_SUCC(write_scope(saved_scope));
}
END_TEST()
