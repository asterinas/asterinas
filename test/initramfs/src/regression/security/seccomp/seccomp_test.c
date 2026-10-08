// SPDX-License-Identifier: MPL-2.0

#include <assert.h>
#include <errno.h>
#include <linux/filter.h>
#include <linux/prctl.h>
#include <linux/seccomp.h>
#include <pthread.h>
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

#ifndef SECCOMP_FILTER_FLAG_TSYNC
#define SECCOMP_FILTER_FLAG_TSYNC (1UL << 0)
#endif

#ifndef SECCOMP_FILTER_FLAG_TSYNC_ESRCH
#define SECCOMP_FILTER_FLAG_TSYNC_ESRCH (1UL << 4)
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

#ifndef BPF_MAXINS
#define BPF_MAXINS 4096
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
		// In strict mode, glibc/musl _exit() calls exit_group, which is forbidden.
		// We must invoke SYS_exit directly.
		syscall(SYS_exit, 0);
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

static void test_filter_verification_failures(void)
{
	// 1. Empty filter (len == 0)
	{
		struct sock_fprog prog = { .len = 0, .filter = NULL };
		errno = 0;
		int ret = seccomp_syscall(SECCOMP_SET_MODE_FILTER, 0, &prog);
		assert(ret == -1 && errno == EINVAL);
	}

	// 2. Filter length exceeding BPF_MAXINS (4096)
	{
		struct sock_fprog prog = { .len = BPF_MAXINS + 1, .filter = (void *)0x1000 };
		errno = 0;
		int ret = seccomp_syscall(SECCOMP_SET_MODE_FILTER, 0, &prog);
		assert(ret == -1 && errno == EINVAL);
	}

	// 3. Program missing RET instruction at the end
	{
		struct sock_filter f[] = {
			BPF_STMT(BPF_LD | BPF_W | BPF_ABS, (uint32_t)offsetof(struct seccomp_data, nr)),
		};
		struct sock_fprog prog = {
			.len = (unsigned short)(sizeof(f) / sizeof(f[0])),
			.filter = f,
		};
		errno = 0;
		int ret = seccomp_syscall(SECCOMP_SET_MODE_FILTER, 0, &prog);
		assert(ret == -1 && errno == EINVAL);
	}

	// 4. Division by zero in ALU (BPF_DIV with k == 0)
	{
		struct sock_filter f[] = {
			BPF_STMT(BPF_ALU | BPF_DIV | BPF_K, 0),
			BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
		};
		struct sock_fprog prog = {
			.len = (unsigned short)(sizeof(f) / sizeof(f[0])),
			.filter = f,
		};
		errno = 0;
		int ret = seccomp_syscall(SECCOMP_SET_MODE_FILTER, 0, &prog);
		assert(ret == -1 && errno == EINVAL);
	}

	// 5. Modulo by zero in ALU (BPF_MOD with k == 0)
	{
		struct sock_filter f[] = {
			BPF_STMT(BPF_ALU | BPF_MOD | BPF_K, 0),
			BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
		};
		struct sock_fprog prog = {
			.len = (unsigned short)(sizeof(f) / sizeof(f[0])),
			.filter = f,
		};
		errno = 0;
		int ret = seccomp_syscall(SECCOMP_SET_MODE_FILTER, 0, &prog);
		assert(ret == -1 && errno == EINVAL);
	}

	// 6. Bit shift >= 32 (BPF_LSH with k == 32)
	{
		struct sock_filter f[] = {
			BPF_STMT(BPF_ALU | BPF_LSH | BPF_K, 32),
			BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
		};
		struct sock_fprog prog = {
			.len = (unsigned short)(sizeof(f) / sizeof(f[0])),
			.filter = f,
		};
		errno = 0;
		int ret = seccomp_syscall(SECCOMP_SET_MODE_FILTER, 0, &prog);
		assert(ret == -1 && errno == EINVAL);
	}

	// 7. Unconditional jump out of bounds (BPF_JA past filter length)
	{
		struct sock_filter f[] = {
			BPF_STMT(BPF_JMP | BPF_JA, 5),
			BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
		};
		struct sock_fprog prog = {
			.len = (unsigned short)(sizeof(f) / sizeof(f[0])),
			.filter = f,
		};
		errno = 0;
		int ret = seccomp_syscall(SECCOMP_SET_MODE_FILTER, 0, &prog);
		assert(ret == -1 && errno == EINVAL);
	}

	// 8. Conditional jump out of bounds (jt/jf past filter length)
	{
		struct sock_filter f[] = {
			BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, 0, 10, 0),
			BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
		};
		struct sock_fprog prog = {
			.len = (unsigned short)(sizeof(f) / sizeof(f[0])),
			.filter = f,
		};
		errno = 0;
		int ret = seccomp_syscall(SECCOMP_SET_MODE_FILTER, 0, &prog);
		assert(ret == -1 && errno == EINVAL);
	}

	// 9. Load uninitialized memory word (BPF_LD | BPF_MEM before ST)
	{
		struct sock_filter f[] = {
			BPF_STMT(BPF_LD | BPF_MEM, 0),
			BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
		};
		struct sock_fprog prog = {
			.len = (unsigned short)(sizeof(f) / sizeof(f[0])),
			.filter = f,
		};
		errno = 0;
		int ret = seccomp_syscall(SECCOMP_SET_MODE_FILTER, 0, &prog);
		assert(ret == -1 && errno == EINVAL);
	}

	// 10. Unaligned seccomp_data access (offset 3 is not 4-byte aligned)
	{
		struct sock_filter f[] = {
			BPF_STMT(BPF_LD | BPF_W | BPF_ABS, 3),
			BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
		};
		struct sock_fprog prog = {
			.len = (unsigned short)(sizeof(f) / sizeof(f[0])),
			.filter = f,
		};
		errno = 0;
		int ret = seccomp_syscall(SECCOMP_SET_MODE_FILTER, 0, &prog);
		assert(ret == -1 && errno == EINVAL);
	}

	// 11. Out-of-bounds seccomp_data access (offset >= sizeof(struct seccomp_data) = 64)
	{
		struct sock_filter f[] = {
			BPF_STMT(BPF_LD | BPF_W | BPF_ABS, 64),
			BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
		};
		struct sock_fprog prog = {
			.len = (unsigned short)(sizeof(f) / sizeof(f[0])),
			.filter = f,
		};
		errno = 0;
		int ret = seccomp_syscall(SECCOMP_SET_MODE_FILTER, 0, &prog);
		assert(ret == -1 && errno == EINVAL);
	}

	// 12. Invalid / unknown BPF opcode (0xffff)
	{
		struct sock_filter f[] = {
			BPF_STMT(0xffff, 0),
			BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
		};
		struct sock_fprog prog = {
			.len = (unsigned short)(sizeof(f) / sizeof(f[0])),
			.filter = f,
		};
		errno = 0;
		int ret = seccomp_syscall(SECCOMP_SET_MODE_FILTER, 0, &prog);
		assert(ret == -1 && errno == EINVAL);
	}

	printf("  [PASS] test_filter_verification_failures\n");
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

		// syscall(SYS_getppid) should fail with EACCES
		// Note: libc's getppid() does not check for errors since POSIX defines getppid
		// as never failing. Calling syscall(SYS_getppid) sets errno and returns -1 on failure.
		errno = 0;
		long ret_val = syscall(SYS_getppid);
		assert(ret_val == -1);
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
		// This must terminate the child with SIGSYS
		getppid();
		_exit(0);
	}

	int status = 0;
	waitpid(pid, &status, 0);
	assert(WIFSIGNALED(status));
	assert(WTERMSIG(status) == SIGSYS);
	printf("  [PASS] test_filter_kill (killed by SIGSYS as expected)\n");
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
			long ret_val = syscall(SYS_getppid);
			assert(ret_val == -1);
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
		long ret_val = syscall(SYS_getppid);
		assert(ret_val == -1);
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

static void *tsync_sibling_thread(void *arg)
{
	int *pipes = (int *)arg;

	// Wait for main thread to install filter with TSYNC
	char buf;
	ssize_t n = read(pipes[0], &buf, 1);
	assert(n == 1);

	// Now try calling SYS_getppid - it must be blocked by the synchronized filter
	errno = 0;
	long ret = syscall(SYS_getppid);
	assert(ret == -1);
	assert(errno == EACCES);

	// Signal main thread that check passed
	n = write(pipes[1], "K", 1);
	assert(n == 1);

	return NULL;
}

static void test_filter_tsync(void)
{
	// Filter: block SYS_getppid with EACCES, allow others
	struct sock_filter filter[] = {
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS, (uint32_t)offsetof(struct seccomp_data, nr)),
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
		int pipe_to_child[2];
		int pipe_to_parent[2];
		assert(pipe(pipe_to_child) == 0);
		assert(pipe(pipe_to_parent) == 0);

		int pipes[2] = { pipe_to_child[0], pipe_to_parent[1] };
		pthread_t tid;
		int prc = pthread_create(&tid, NULL, tsync_sibling_thread, pipes);
		assert(prc == 0);

		// Install filter with TSYNC
		int ret = seccomp_syscall(SECCOMP_SET_MODE_FILTER, SECCOMP_FILTER_FLAG_TSYNC, &prog);
		if (ret != 0) {
			perror("seccomp tsync failed");
			exit(1);
		}

		// Tell sibling thread that filter is installed
		ssize_t n = write(pipe_to_child[1], "G", 1);
		assert(n == 1);

		// Wait for sibling thread confirmation
		char ack;
		n = read(pipe_to_parent[0], &ack, 1);
		assert(n == 1);
		assert(ack == 'K');

		pthread_join(tid, NULL);
		close(pipe_to_child[0]);
		close(pipe_to_child[1]);
		close(pipe_to_parent[0]);
		close(pipe_to_parent[1]);

		_exit(0);
	}

	int status = 0;
	waitpid(pid, &status, 0);
	assert(WIFEXITED(status));
	assert(WEXITSTATUS(status) == 0);
	printf("  [PASS] test_filter_tsync\n");
}

static void *tsync_conflict_sibling_thread(void *arg)
{
	int *pipes = (int *)arg;

	// Sibling thread installs its own filter first
	struct sock_filter filter[] = {
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
	};
	struct sock_fprog prog = {
		.len = 1,
		.filter = filter,
	};

	int ret = seccomp_syscall(SECCOMP_SET_MODE_FILTER, 0, &prog);
	assert(ret == 0);

	// Notify main thread that filter is installed
	ssize_t n = write(pipes[1], "R", 1);
	assert(n == 1);

	// Wait for main thread to finish its conflict test
	char buf;
	n = read(pipes[0], &buf, 1);
	assert(n == 1);

	return NULL;
}

static void test_filter_tsync_conflict(void)
{
	struct sock_filter filter[] = {
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
	};
	struct sock_fprog prog = {
		.len = 1,
		.filter = filter,
	};

	pid_t pid = fork();
	assert(pid >= 0);

	if (pid == 0) {
		int pipe_to_child[2];
		int pipe_to_parent[2];
		assert(pipe(pipe_to_child) == 0);
		assert(pipe(pipe_to_parent) == 0);

		int pipes[2] = { pipe_to_child[0], pipe_to_parent[1] };
		pthread_t tid;
		int prc = pthread_create(&tid, NULL, tsync_conflict_sibling_thread, pipes);
		assert(prc == 0);

		// Wait for sibling thread to install its diverged filter
		char ready;
		ssize_t n = read(pipe_to_parent[0], &ready, 1);
		assert(n == 1);

		// Now attempting TSYNC with TSYNC_ESRCH must fail with ESRCH
		errno = 0;
		int ret = seccomp_syscall(SECCOMP_SET_MODE_FILTER,
			SECCOMP_FILTER_FLAG_TSYNC | SECCOMP_FILTER_FLAG_TSYNC_ESRCH, &prog);
		assert(ret == -1);
		assert(errno == ESRCH);

		// Attempting TSYNC without TSYNC_ESRCH must return the conflicting thread TID (> 0)
		ret = seccomp_syscall(SECCOMP_SET_MODE_FILTER, SECCOMP_FILTER_FLAG_TSYNC, &prog);
		assert(ret > 0);

		// Release sibling thread
		n = write(pipe_to_child[1], "D", 1);
		assert(n == 1);

		pthread_join(tid, NULL);
		close(pipe_to_child[0]);
		close(pipe_to_child[1]);
		close(pipe_to_parent[0]);
		close(pipe_to_parent[1]);

		_exit(0);
	}

	int status = 0;
	waitpid(pid, &status, 0);
	assert(WIFEXITED(status));
	assert(WEXITSTATUS(status) == 0);
	printf("  [PASS] test_filter_tsync_conflict\n");
}

int main(void)
{
	printf("Starting seccomp regression tests...\n");
	test_initial_state();
	test_strict_mode_allowed();
	test_strict_mode_killed();
	test_filter_verification_failures();
	test_filter_allow_all();
	test_filter_errno();
	test_filter_kill();
	test_filter_inheritance();
	test_filter_chaining();
	test_filter_tsync();
	test_filter_tsync_conflict();
	printf("All seccomp regression tests passed successfully!\n");
	return 0;
}
