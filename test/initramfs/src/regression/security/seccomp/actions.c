// SPDX-License-Identifier: MPL-2.0

#include "seccomp_common.h"

static void errno_boundaries(void)
{
	no_new_privs();
	deny_syscall(SYS_getppid, SECCOMP_RET_ERRNO);
	CHECK_WITH(syscall(SYS_getppid), _ret == 0);
	deny_syscall(SYS_getppid, SECCOMP_RET_ERRNO | 0xffff);
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == 4095);
	deny_syscall(SYS_getppid, SECCOMP_RET_ERRNO | 4094);
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == 4094);
}

static void unknown_syscall_is_filtered(void)
{
	no_new_privs();
	deny_syscall(-1, SECCOMP_RET_ERRNO | EBUSY);
	CHECK_WITH(syscall(-1), _ret == -1 && errno == EBUSY);
}

static void rejected_write_has_no_side_effect(void)
{
	int fds[2];
	CHECK(pipe2(fds, O_NONBLOCK));
	no_new_privs();
	deny_syscall(SYS_write, SECCOMP_RET_ERRNO | EACCES);
	errno = 0;
	long ret = syscall(SYS_write, fds[1], "x", 1);
	int saved_errno = errno;
	char byte;
	long read_ret = read(fds[0], &byte, 1);
	int read_errno = errno;
	close(fds[0]);
	close(fds[1]);
	/* Reporting a failure must not require the blocked write syscall. */
	_exit(ret == -1 && saved_errno == EACCES && read_ret == -1 &&
			      read_errno == EAGAIN ?
		      0 :
		      1);
}

static void kill_process(void)
{
	no_new_privs();
	deny_syscall(SYS_getppid, SECCOMP_RET_KILL_PROCESS);
	syscall(SYS_getppid);
	_exit(1);
}

static void unknown_action(void)
{
	no_new_privs();
	deny_syscall(SYS_getppid, 0x12340000);
	syscall(SYS_getppid);
	_exit(1);
}

static void *kill_worker(void *arg)
{
	(void)arg;
	kill_process();
	return NULL;
}

static void kill_all_threads(void)
{
	pthread_t thread;
	CHECK(pthread_create(&thread, NULL, kill_worker, NULL));
	CHECK(pthread_join(thread, NULL));
	_exit(1);
}

FN_TEST(actions)
{
	TEST_RES(run_child(errno_boundaries, 0), _ret == 0);
	TEST_RES(run_child(unknown_syscall_is_filtered, 0), _ret == 0);
	TEST_RES(run_child(rejected_write_has_no_side_effect, 0), _ret == 0);
	TEST_RES(run_child(kill_process, SIGSYS), _ret == 0);
	TEST_RES(run_child(unknown_action, SIGSYS), _ret == 0);
	TEST_RES(run_child(kill_all_threads, SIGSYS), _ret == 0);
}
END_TEST()
