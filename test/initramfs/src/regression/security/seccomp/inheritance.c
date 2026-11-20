// SPDX-License-Identifier: MPL-2.0

#include "seccomp_common.h"
#include <linux/sched.h>

__attribute__((constructor(101))) static void exec_probe(void)
{
	if (!getenv("SECCOMP_EXEC_PROBE"))
		return;
	CHECK_WITH(prctl(PR_GET_SECCOMP), _ret == SECCOMP_MODE_FILTER);
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
	_exit(0);
}

static void fork_inherits_and_append_is_private(void)
{
	no_new_privs();
	deny_syscall(SYS_getppid, SECCOMP_RET_ERRNO | EBUSY);
	pid_t pid = CHECK(fork());
	if (pid == 0) {
		CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
		deny_syscall(SYS_getpid, SECCOMP_RET_ERRNO | EACCES);
		CHECK_WITH(syscall(SYS_getpid), _ret == -1 && errno == EACCES);
		_exit(0);
	}
	int status;
	CHECK_WITH(waitpid(pid, &status, 0), _ret == pid && WIFEXITED(status) &&
						     WEXITSTATUS(status) == 0);
	CHECK(syscall(SYS_getpid));
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
}

static pthread_barrier_t barrier;

static void *existing_thread(void *arg)
{
	(void)arg;
	CHECK_WITH(pthread_barrier_wait(&barrier),
		   _ret == 0 || _ret == PTHREAD_BARRIER_SERIAL_THREAD);
	CHECK(syscall(SYS_getppid));
	return NULL;
}

static void *inherited_thread(void *arg)
{
	(void)arg;
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
	return NULL;
}

static void thread_heads_are_independent(void)
{
	no_new_privs();
	pthread_t old_thread, new_thread;
	CHECK(pthread_barrier_init(&barrier, NULL, 2));
	CHECK(pthread_create(&old_thread, NULL, existing_thread, NULL));
	deny_syscall(SYS_getppid, SECCOMP_RET_ERRNO | EBUSY);
	CHECK_WITH(pthread_barrier_wait(&barrier),
		   _ret == 0 || _ret == PTHREAD_BARRIER_SERIAL_THREAD);
	CHECK(pthread_join(old_thread, NULL));
	CHECK(pthread_barrier_destroy(&barrier));
	CHECK(pthread_create(&new_thread, NULL, inherited_thread, NULL));
	CHECK(pthread_join(new_thread, NULL));
}

static void clone3_inherits(void)
{
	no_new_privs();
	deny_syscall(SYS_getppid, SECCOMP_RET_ERRNO | EBUSY);
	struct clone_args args = { .exit_signal = SIGCHLD };
	pid_t pid = CHECK(syscall(SYS_clone3, &args, sizeof(args)));
	if (pid == 0) {
		CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
		_exit(0);
	}
	int status;
	CHECK_WITH(waitpid(pid, &status, 0), _ret == pid && WIFEXITED(status) &&
						     WEXITSTATUS(status) == 0);
}

static void execveat_with_policy(void)
{
	int fd = CHECK(open("/proc/self/exe", O_RDONLY));
	no_new_privs();
	deny_syscall(SYS_getppid, SECCOMP_RET_ERRNO | EBUSY);
	char *const argv[] = { "seccomp-execveat-probe", NULL };
	char *const envp[] = { "SECCOMP_EXEC_PROBE=1", NULL };
	CHECK(syscall(SYS_execveat, fd, "", argv, envp, AT_EMPTY_PATH));
	_exit(1);
}

static void exec_with_policy(void)
{
	no_new_privs();
	deny_syscall(SYS_getppid, SECCOMP_RET_ERRNO | EBUSY);
	char *const argv[] = { "seccomp-exec-probe", NULL };
	char *const envp[] = { "SECCOMP_EXEC_PROBE=1", NULL };
	CHECK(execve("/proc/self/exe", argv, envp));
	_exit(1);
}

static void *exec_worker(void *arg)
{
	(void)arg;
	exec_with_policy();
	return NULL;
}

static void nonleader_exec_preserves_policy(void)
{
	pthread_t thread;
	CHECK(pthread_create(&thread, NULL, exec_worker, NULL));
	CHECK(pthread_join(thread, NULL));
	_exit(1);
}

FN_TEST(inheritance)
{
	TEST_RES(run_child(fork_inherits_and_append_is_private, 0), _ret == 0);
	TEST_RES(run_child(thread_heads_are_independent, 0), _ret == 0);
	TEST_RES(run_child(clone3_inherits, 0), _ret == 0);
	TEST_RES(run_child(exec_with_policy, 0), _ret == 0);
	TEST_RES(run_child(execveat_with_policy, 0), _ret == 0);
	TEST_RES(run_child(nonleader_exec_preserves_policy, 0), _ret == 0);
}
END_TEST()
