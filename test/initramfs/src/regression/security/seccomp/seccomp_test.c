// SPDX-License-Identifier: MPL-2.0

#include <assert.h>
#include <errno.h>
#include <linux/filter.h>
#include <linux/prctl.h>
#include <linux/seccomp.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#ifndef SYS_seccomp
#define SYS_seccomp 317
#endif

#ifndef PR_GET_SECCOMP
#define PR_GET_SECCOMP 21
#endif

#ifndef PR_SET_SECCOMP
#define PR_SET_SECCOMP 22
#endif

#ifndef SECCOMP_SET_MODE_STRICT
#define SECCOMP_SET_MODE_STRICT 0
#endif

#ifndef SECCOMP_SET_MODE_FILTER
#define SECCOMP_SET_MODE_FILTER 1
#endif

#ifndef SECCOMP_RET_KILL
#define SECCOMP_RET_KILL 0x00000000U
#endif

#ifndef SECCOMP_RET_ERRNO
#define SECCOMP_RET_ERRNO 0x00050000U
#endif

#ifndef SECCOMP_RET_ALLOW
#define SECCOMP_RET_ALLOW 0x7fff0000U
#endif

static int seccomp_syscall(unsigned int op, unsigned int flags, void *args)
{
	return syscall(SYS_seccomp, op, flags, args);
}

static void test_initial_state(void)
{
	int mode = prctl(PR_GET_SECCOMP, 0, 0, 0, 0);
	assert(mode == 0);
	printf("  [PASS] test_initial_state (mode = %d)\n", mode);
}

static void test_strict_mode_allowed(void)
{
	pid_t pid = fork();
	assert(pid >= 0);

	if (pid == 0) {
		int ret = seccomp_syscall(SECCOMP_SET_MODE_STRICT, 0, NULL);
		if (ret != 0) {
			perror("seccomp strict failed");
			exit(1);
		}
		// In strict mode: read, write, exit, rt_sigreturn are allowed.
		char msg[] = "    strict mode write succeeded\n";
		ssize_t written = write(1, msg, sizeof(msg) - 1);
		(void)written;
		_exit(0);
	}

	int status = 0;
	waitpid(pid, &status, 0);
	assert(WIFEXITED(status));
	assert(WEXITSTATUS(status) == 0);
	printf("  [PASS] test_strict_mode_allowed\n");
}

static void test_strict_mode_killed(void)
{
	pid_t pid = fork();
	assert(pid >= 0);

	if (pid == 0) {
		int ret = seccomp_syscall(SECCOMP_SET_MODE_STRICT, 0, NULL);
		if (ret != 0) {
			perror("seccomp strict failed");
			exit(1);
		}
		// getpid is forbidden in strict mode and must trigger SIGKILL.
		getpid();
		_exit(0);
	}

	int status = 0;
	waitpid(pid, &status, 0);
	assert(WIFSIGNALED(status));
	assert(WTERMSIG(status) == SIGKILL);
	printf("  [PASS] test_strict_mode_killed (killed by SIGKILL as expected)\n");
}

static void test_filter_allow_all(void)
{
	struct sock_filter filter[] = {
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
	};
	struct sock_fprog prog = {
		.len = (unsigned short)(sizeof(filter) / sizeof(filter[0])),
		.filter = filter,
	};

	pid_t pid = fork();
	assert(pid >= 0);

	if (pid == 0) {
		int ret = seccomp_syscall(SECCOMP_SET_MODE_FILTER, 0, &prog);
		if (ret != 0) {
			perror("seccomp filter allow failed");
			exit(1);
		}
		int mode = prctl(PR_GET_SECCOMP, 0, 0, 0, 0);
		assert(mode == 2);
		pid_t my_pid = getpid();
		assert(my_pid > 0);
		_exit(0);
	}

	int status = 0;
	waitpid(pid, &status, 0);
	assert(WIFEXITED(status));
	assert(WEXITSTATUS(status) == 0);
	printf("  [PASS] test_filter_allow_all\n");
}

static void test_filter_errno(void)
{
	// Filter: if syscall == SYS_getppid, return ERRNO(EACCES); else ALLOW.
	struct sock_filter filter[] = {
		// Load syscall number: offset 0 in seccomp_data
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS, (uint32_t)offsetof(struct seccomp_data, nr)),
		// If syscall == SYS_getppid, jump +1 (to ERRNO), else +0 (to ALLOW)
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, (uint32_t)SYS_getppid, 1, 0),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ERRNO | (EACCES & 0xffff)),
	};
	struct sock_fprog prog = {
		.len = (unsigned short)(sizeof(filter) / sizeof(filter[0])),
		.filter = filter,
	};

	pid_t pid = fork();
	assert(pid >= 0);

	if (pid == 0) {
		int ret = seccomp_syscall(SECCOMP_SET_MODE_FILTER, 0, &prog);
		if (ret != 0) {
			perror("seccomp filter errno failed");
			exit(1);
		}

		// getpid() should be allowed
		pid_t my_pid = getpid();
		assert(my_pid > 0);

		// getppid() should fail with EACCES
		errno = 0;
		pid_t parent_pid = getppid();
		assert(parent_pid == -1);
		assert(errno == EACCES);

		_exit(0);
	}

	int status = 0;
	waitpid(pid, &status, 0);
	assert(WIFEXITED(status));
	assert(WEXITSTATUS(status) == 0);
	printf("  [PASS] test_filter_errno\n");
}

static void test_filter_kill(void)
{
	// Filter: if syscall == SYS_getppid, return KILL; else ALLOW.
	struct sock_filter filter[] = {
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS, (uint32_t)offsetof(struct seccomp_data, nr)),
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, (uint32_t)SYS_getppid, 0, 1),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_KILL),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
	};
	struct sock_fprog prog = {
		.len = (unsigned short)(sizeof(filter) / sizeof(filter[0])),
		.filter = filter,
	};

	pid_t pid = fork();
	assert(pid >= 0);

	if (pid == 0) {
		int ret = seccomp_syscall(SECCOMP_SET_MODE_FILTER, 0, &prog);
		if (ret != 0) {
			perror("seccomp filter kill failed");
			exit(1);
		}
		// This must terminate the child with SIGKILL
		getppid();
		_exit(0);
	}

	int status = 0;
	waitpid(pid, &status, 0);
	assert(WIFSIGNALED(status));
	assert(WTERMSIG(status) == SIGKILL);
	printf("  [PASS] test_filter_kill (killed by SIGKILL as expected)\n");
}

static void test_filter_inheritance(void)
{
	// Filter: block SYS_getppid with EPERM.
	struct sock_filter filter[] = {
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS, (uint32_t)offsetof(struct seccomp_data, nr)),
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, (uint32_t)SYS_getppid, 0, 1),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ERRNO | (EPERM & 0xffff)),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
	};
	struct sock_fprog prog = {
		.len = (unsigned short)(sizeof(filter) / sizeof(filter[0])),
		.filter = filter,
	};

	pid_t parent_pid = fork();
	assert(parent_pid >= 0);

	if (parent_pid == 0) {
		// Child 1 installs the filter
		int ret = seccomp_syscall(SECCOMP_SET_MODE_FILTER, 0, &prog);
		assert(ret == 0);

		// Child 1 forks grandchild
		pid_t grandchild = fork();
		assert(grandchild >= 0);

		if (grandchild == 0) {
			// Grandchild must inherit seccomp filter!
			errno = 0;
			pid_t res = getppid();
			assert(res == -1);
			assert(errno == EPERM);
			_exit(0);
		}

		int status = 0;
		waitpid(grandchild, &status, 0);
		assert(WIFEXITED(status));
		assert(WEXITSTATUS(status) == 0);
		_exit(0);
	}

	int status = 0;
	waitpid(parent_pid, &status, 0);
	assert(WIFEXITED(status));
	assert(WEXITSTATUS(status) == 0);
	printf("  [PASS] test_filter_inheritance\n");
}

static void test_filter_chaining(void)
{
	// Filter 1: Allow all
	struct sock_filter f1[] = {
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
	};
	struct sock_fprog p1 = {
		.len = 1,
		.filter = f1,
	};

	// Filter 2: Block getppid with EACCES
	struct sock_filter f2[] = {
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS, (uint32_t)offsetof(struct seccomp_data, nr)),
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, (uint32_t)SYS_getppid, 0, 1),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ERRNO | (EACCES & 0xffff)),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
	};
	struct sock_fprog p2 = {
		.len = 4,
		.filter = f2,
	};

	pid_t pid = fork();
	assert(pid >= 0);

	if (pid == 0) {
		// Install filter 1
		int ret = seccomp_syscall(SECCOMP_SET_MODE_FILTER, 0, &p1);
		assert(ret == 0);
		// Install filter 2 (chained on top of filter 1)
		ret = seccomp_syscall(SECCOMP_SET_MODE_FILTER, 0, &p2);
		assert(ret == 0);

		// getppid should be blocked by filter 2 (most restrictive wins)
		errno = 0;
		pid_t res = getppid();
		assert(res == -1);
		assert(errno == EACCES);

		// other syscalls should succeed
		pid_t my_pid = getpid();
		assert(my_pid > 0);

		_exit(0);
	}

	int status = 0;
	waitpid(pid, &status, 0);
	assert(WIFEXITED(status));
	assert(WEXITSTATUS(status) == 0);
	printf("  [PASS] test_filter_chaining\n");
}

int main(void)
{
	printf("Starting seccomp regression tests...\n");
	test_initial_state();
	test_strict_mode_allowed();
	test_strict_mode_killed();
	test_filter_allow_all();
	test_filter_errno();
	test_filter_kill();
	test_filter_inheritance();
	test_filter_chaining();
	printf("All seccomp regression tests passed successfully!\n");
	return 0;
}
