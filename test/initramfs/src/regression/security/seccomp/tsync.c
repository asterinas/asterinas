// SPDX-License-Identifier: MPL-2.0

#include "seccomp_common.h"
#include <stdatomic.h>
#include <sched.h>

static pthread_barrier_t barrier;
static atomic_int done;
static pid_t divergent_tid;

static void rendezvous(void)
{
	CHECK_WITH(pthread_barrier_wait(&barrier),
		   _ret == 0 || _ret == PTHREAD_BARRIER_SERIAL_THREAD);
}

static int sync_filter(unsigned int flags)
{
	struct sock_filter code[] = {
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS,
			 offsetof(struct seccomp_data, nr)),
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_getppid, 0, 1),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ERRNO | EBUSY),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
	};
	struct sock_fprog prog = { .len = 4, .filter = code };
	return syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER, flags, &prog);
}

static void check_synced(void)
{
	CHECK_WITH(prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), _ret == 1);
	CHECK_WITH(prctl(PR_GET_SECCOMP), _ret == SECCOMP_MODE_FILTER);
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
}

static void *sync_target(void *arg)
{
	rendezvous();
	rendezvous();
	check_synced();
	if (arg) {
		CHECK_WITH(syscall(SYS_getuid), _ret == -1 && errno == EACCES);
		CHECK_WITH(syscall(SYS_getgid), _ret == -1 && errno == EPERM);
	}
	return NULL;
}

static void disabled_target_and_nnp(void)
{
	pthread_t thread;
	CHECK(pthread_barrier_init(&barrier, NULL, 2));
	CHECK_WITH(prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), _ret == 0);
	CHECK(pthread_create(&thread, NULL, sync_target, NULL));
	rendezvous();
	no_new_privs();
	CHECK_WITH(sync_filter(SECCOMP_FILTER_FLAG_TSYNC), _ret == 0);
	check_synced();
	rendezvous();
	CHECK(pthread_join(thread, NULL));
	CHECK(pthread_barrier_destroy(&barrier));
}

static void ancestor_target(void)
{
	pthread_t thread;
	no_new_privs();
	deny_syscall(SYS_getuid, SECCOMP_RET_ERRNO | EACCES);
	CHECK(pthread_barrier_init(&barrier, NULL, 2));
	CHECK(pthread_create(&thread, NULL, sync_target, (void *)1));
	rendezvous();
	deny_syscall(SYS_getgid, SECCOMP_RET_ERRNO | EPERM);
	CHECK_WITH(sync_filter(SECCOMP_FILTER_FLAG_TSYNC), _ret == 0);
	check_synced();
	CHECK_WITH(syscall(SYS_getuid), _ret == -1 && errno == EACCES);
	CHECK_WITH(syscall(SYS_getgid), _ret == -1 && errno == EPERM);
	/* A second sync exercises identical heads. */
	CHECK_WITH(sync_filter(SECCOMP_FILTER_FLAG_TSYNC |
			       SECCOMP_FILTER_FLAG_TSYNC_ESRCH),
		   _ret == 0);
	rendezvous();
	CHECK(pthread_join(thread, NULL));
	CHECK(pthread_barrier_destroy(&barrier));
}

static void install_allow(void)
{
	struct sock_filter code = BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW);
	CHECK(install(&code, 1));
}

static void *divergent_target(void *arg)
{
	(void)arg;
	divergent_tid = syscall(SYS_gettid);
	no_new_privs();
	install_allow();
	rendezvous();
	rendezvous();
	CHECK(syscall(SYS_getppid));
	return NULL;
}

static void *unchanged_target(void *arg)
{
	(void)arg;
	rendezvous();
	rendezvous();
	CHECK_WITH(prctl(PR_GET_SECCOMP), _ret == 0);
	CHECK_WITH(prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), _ret == 0);
	CHECK(syscall(SYS_getppid));
	return NULL;
}

static void failed_sync_is_atomic(void)
{
	pthread_t untouched, divergent;
	CHECK(pthread_barrier_init(&barrier, NULL, 3));
	/* This target comes before the failing target in the task list. */
	CHECK(pthread_create(&untouched, NULL, unchanged_target, NULL));
	CHECK(pthread_create(&divergent, NULL, divergent_target, NULL));
	rendezvous();
	no_new_privs();
	/* Identical bytecode installed independently is not an ancestor. */
	install_allow();
	CHECK_WITH(sync_filter(SECCOMP_FILTER_FLAG_TSYNC),
		   _ret == divergent_tid);
	CHECK_WITH(sync_filter(SECCOMP_FILTER_FLAG_TSYNC |
			       SECCOMP_FILTER_FLAG_TSYNC_ESRCH),
		   _ret == -1 && errno == ESRCH);
	CHECK(syscall(SYS_getppid));
	rendezvous();
	CHECK(pthread_join(untouched, NULL));
	CHECK(pthread_join(divergent, NULL));
	/* Exited divergent threads no longer obstruct synchronization. */
	CHECK_WITH(sync_filter(SECCOMP_FILTER_FLAG_TSYNC), _ret == 0);
	check_synced();
	CHECK(pthread_barrier_destroy(&barrier));
}

static void esrch_alone_is_local(void)
{
	pthread_t thread;
	CHECK(pthread_barrier_init(&barrier, NULL, 2));
	CHECK(pthread_create(&thread, NULL, unchanged_target, NULL));
	rendezvous();
	no_new_privs();
	CHECK_WITH(sync_filter(SECCOMP_FILTER_FLAG_TSYNC_ESRCH), _ret == 0);
	check_synced();
	rendezvous();
	CHECK(pthread_join(thread, NULL));
	CHECK(pthread_barrier_destroy(&barrier));
}

static void invalid_flags_and_program(void)
{
	no_new_privs();
	CHECK_WITH(sync_filter(SECCOMP_FILTER_FLAG_TSYNC | 0x80000000),
		   _ret == -1 && errno == EINVAL);
	CHECK_WITH(syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER,
			   SECCOMP_FILTER_FLAG_TSYNC, NULL),
		   _ret == -1 && errno == EFAULT);
	CHECK_WITH(prctl(PR_GET_SECCOMP), _ret == 0);
}

static void *append_target(void *arg)
{
	(void)arg;
	divergent_tid = syscall(SYS_gettid);
	rendezvous();
	deny_syscall(SYS_getuid, SECCOMP_RET_ERRNO | EACCES);
	rendezvous();
	CHECK_WITH(syscall(SYS_getuid), _ret == -1 && errno == EACCES);
	if (atomic_load(&done))
		check_synced();
	else
		CHECK(syscall(SYS_getppid));
	return NULL;
}

static void concurrent_append(void)
{
	pthread_t thread;
	no_new_privs();
	install_allow();
	CHECK(pthread_barrier_init(&barrier, NULL, 2));
	CHECK(pthread_create(&thread, NULL, append_target, NULL));
	rendezvous();
	int ret = sync_filter(SECCOMP_FILTER_FLAG_TSYNC);
	CHECK_WITH(ret, _ret == 0 || _ret == divergent_tid);
	atomic_store(&done, ret == 0);
	rendezvous();
	CHECK(pthread_join(thread, NULL));
	CHECK(syscall(SYS_getuid));
	if (ret == 0)
		check_synced();
	else
		CHECK(syscall(SYS_getppid));
	CHECK(pthread_barrier_destroy(&barrier));
}

static void *cloned_target(void *arg)
{
	(void)arg;
	while (!atomic_load(&done))
		sched_yield();
	check_synced();
	return NULL;
}

static void *clone_worker(void *arg)
{
	(void)arg;
	pthread_t threads[16];
	rendezvous();
	for (size_t i = 0; i < 16; ++i)
		CHECK(pthread_create(&threads[i], NULL, cloned_target, NULL));
	for (size_t i = 0; i < 16; ++i)
		CHECK(pthread_join(threads[i], NULL));
	check_synced();
	return NULL;
}

static void clone_race(void)
{
	pthread_t thread;
	CHECK(pthread_barrier_init(&barrier, NULL, 2));
	CHECK(pthread_create(&thread, NULL, clone_worker, NULL));
	no_new_privs();
	rendezvous();
	CHECK_WITH(sync_filter(SECCOMP_FILTER_FLAG_TSYNC), _ret == 0);
	atomic_store(&done, 1);
	CHECK(pthread_join(thread, NULL));
	CHECK(pthread_barrier_destroy(&barrier));
}

static void *fork_worker(void *arg)
{
	(void)arg;
	rendezvous();
	for (int i = 0; i < 16; ++i) {
		pid_t pid = CHECK(fork());
		if (pid == 0) {
			int mode = CHECK(prctl(PR_GET_SECCOMP));
			if (mode == SECCOMP_MODE_FILTER)
				check_synced();
			else {
				CHECK_WITH(mode, _ret == 0);
				CHECK_WITH(prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0,
						 0),
					   _ret == 0);
				CHECK(syscall(SYS_getppid));
			}
			_exit(0);
		}
		int status;
		CHECK_WITH(waitpid(pid, &status, 0),
			   _ret == pid && WIFEXITED(status) &&
				   WEXITSTATUS(status) == 0);
	}
	return NULL;
}

static void fork_race(void)
{
	pthread_t thread;
	CHECK(pthread_barrier_init(&barrier, NULL, 2));
	CHECK(pthread_create(&thread, NULL, fork_worker, NULL));
	no_new_privs();
	rendezvous();
	CHECK_WITH(sync_filter(SECCOMP_FILTER_FLAG_TSYNC), _ret == 0);
	CHECK(pthread_join(thread, NULL));
	CHECK(pthread_barrier_destroy(&barrier));
}

__attribute__((constructor(101))) static void exec_probe(void)
{
	if (!getenv("SECCOMP_TSYNC_EXEC_PROBE"))
		return;
	int mode = CHECK(prctl(PR_GET_SECCOMP));
	if (mode == SECCOMP_MODE_FILTER)
		check_synced();
	else
		CHECK_WITH(mode, _ret == 0);
	_exit(0);
}

static void *exec_sync_worker(void *arg)
{
	(void)arg;
	no_new_privs();
	rendezvous();
	/* Keep contending with exec until exec kills this thread. */
	for (;;)
		CHECK_WITH(sync_filter(SECCOMP_FILTER_FLAG_TSYNC), _ret == 0);
}

static void exec_race(void)
{
	pthread_t thread;
	CHECK(pthread_barrier_init(&barrier, NULL, 2));
	CHECK(pthread_create(&thread, NULL, exec_sync_worker, NULL));
	rendezvous();
	char *const argv[] = { "tsync-exec-probe", NULL };
	char *const envp[] = { "SECCOMP_TSYNC_EXEC_PROBE=1", NULL };
	CHECK(execve("/proc/self/exe", argv, envp));
	_exit(1);
}

FN_TEST(tsync)
{
	alarm(120);
	TEST_RES(run_child(disabled_target_and_nnp, 0), _ret == 0);
	TEST_RES(run_child(ancestor_target, 0), _ret == 0);
	TEST_RES(run_child(failed_sync_is_atomic, 0), _ret == 0);
	TEST_RES(run_child(esrch_alone_is_local, 0), _ret == 0);
	TEST_RES(run_child(invalid_flags_and_program, 0), _ret == 0);
	for (int i = 0; i < 8; ++i) {
		TEST_RES(run_child(concurrent_append, 0), _ret == 0);
		TEST_RES(run_child(clone_race, 0), _ret == 0);
		TEST_RES(run_child(fork_race, 0), _ret == 0);
		TEST_RES(run_child(exec_race, 0), _ret == 0);
	}
	alarm(0);
}
END_TEST()
