// SPDX-License-Identifier: MPL-2.0

#include "seccomp_common.h"
#include <stdatomic.h>
#include <sched.h>

/* libc exit and pthread teardown use syscalls forbidden by strict mode. */
static void finish(int status) __attribute__((noreturn));
static void finish(int status)
{
	syscall(SYS_exit, status);
	__builtin_unreachable();
}

static void require(int condition)
{
	if (!condition)
		finish(1);
}

static void enter_strict(void)
{
	require(syscall(SYS_seccomp, SECCOMP_SET_MODE_STRICT, 0, NULL) == 0);
}

static void invalid_arguments(void)
{
	CHECK_WITH(syscall(SYS_seccomp, SECCOMP_SET_MODE_STRICT, 1, NULL),
		   _ret == -1 && errno == EINVAL);
	CHECK_WITH(syscall(SYS_seccomp, SECCOMP_SET_MODE_STRICT, 0, (void *)1),
		   _ret == -1 && errno == EINVAL);
	CHECK_WITH(prctl(PR_SET_SECCOMP, UINT64_C(0x100000001), 0),
		   _ret == -1 && errno == EINVAL);
	CHECK_WITH(prctl(PR_SET_SECCOMP, SECCOMP_MODE_DISABLED, 0),
		   _ret == -1 && errno == EINVAL);
	CHECK_WITH(prctl(PR_GET_SECCOMP), _ret == SECCOMP_MODE_DISABLED);
}

static void filter_cannot_become_strict(void)
{
	no_new_privs();
	deny_syscall(SYS_getppid, SECCOMP_RET_ERRNO | EBUSY);
	CHECK_WITH(syscall(SYS_seccomp, SECCOMP_SET_MODE_STRICT, 0, NULL),
		   _ret == -1 && errno == EINVAL);
	CHECK_WITH(prctl(PR_SET_SECCOMP, SECCOMP_MODE_STRICT, (void *)1),
		   _ret == -1 && errno == EINVAL);
	CHECK_WITH(prctl(PR_GET_SECCOMP), _ret == SECCOMP_MODE_FILTER);
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
}

static int use_prctl, set_nnp;
static void unprivileged_io(void)
{
	int fds[2];
	CHECK(pipe(fds));
	if (geteuid() == 0)
		CHECK(setuid(65534));
	CHECK_WITH(prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), _ret == 0);
	if (set_nnp)
		no_new_privs();
	if (use_prctl)
		require(prctl(PR_SET_SECCOMP, SECCOMP_MODE_STRICT, (void *)1,
			      ~0UL, ~0UL) == 0);
	else
		enter_strict();
	char byte = 's', received = 0;
	require(syscall(SYS_write, fds[1], &byte, 1) == 1);
	require(syscall(SYS_read, fds[0], &received, 1) == 1);
	require(received == byte);
	finish(0);
}

static int forbidden_nr;
static void forbidden_call(void)
{
	enter_strict();
	syscall(forbidden_nr, 0, 0, 0, 0, 0, 0);
	finish(1);
}

static int ready_pipe[2], release_pipe[2];
static atomic_int escaped;
static pid_t strict_tid;

static void setup_pipes(void)
{
	CHECK(pipe(ready_pipe));
	CHECK(pipe(release_pipe));
}

static void close_pipes(void)
{
	CHECK(close(ready_pipe[0]));
	CHECK(close(ready_pipe[1]));
	CHECK(close(release_pipe[0]));
	CHECK(close(release_pipe[1]));
}

static void *strict_target(void *unused)
{
	(void)unused;
	strict_tid = syscall(SYS_gettid);
	enter_strict();
	char byte = 'r';
	require(syscall(SYS_write, ready_pipe[1], &byte, 1) == 1);
	require(syscall(SYS_read, release_pipe[0], &byte, 1) == 1);
	syscall(SYS_getpid);
	atomic_store(&escaped, 1);
	finish(1);
}

static pthread_barrier_t observer_barrier;
static void *unchanged_observer(void *unused)
{
	(void)unused;
	CHECK_WITH(pthread_barrier_wait(&observer_barrier),
		   _ret == 0 || _ret == PTHREAD_BARRIER_SERIAL_THREAD);
	CHECK_WITH(prctl(PR_GET_SECCOMP), _ret == SECCOMP_MODE_DISABLED);
	CHECK_WITH(prctl(PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0), _ret == 0);
	CHECK(syscall(SYS_getppid));
	return NULL;
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

static int caller_has_filter;
static void strict_blocks_sync(void)
{
	pthread_t strict, observer;
	setup_pipes();
	CHECK(pthread_barrier_init(&observer_barrier, NULL, 2));
	/* The eligible observer precedes the strict target in the task list. */
	CHECK(pthread_create(&observer, NULL, unchanged_observer, NULL));
	CHECK(pthread_create(&strict, NULL, strict_target, NULL));
	char byte;
	CHECK_WITH(read(ready_pipe[0], &byte, 1), _ret == 1);
	CHECK_WITH(prctl(PR_GET_SECCOMP), _ret == SECCOMP_MODE_DISABLED);
	no_new_privs();
	if (caller_has_filter)
		deny_syscall(SYS_getuid, SECCOMP_RET_ERRNO | EACCES);
	CHECK_WITH(sync_filter(SECCOMP_FILTER_FLAG_TSYNC), _ret == strict_tid);
	CHECK_WITH(sync_filter(SECCOMP_FILTER_FLAG_TSYNC |
			       SECCOMP_FILTER_FLAG_TSYNC_ESRCH),
		   _ret == -1 && errno == ESRCH);
	CHECK_WITH(prctl(PR_GET_SECCOMP),
		   _ret == (caller_has_filter ? SECCOMP_MODE_FILTER : 0));
	CHECK(syscall(SYS_getppid));
	if (caller_has_filter)
		CHECK_WITH(syscall(SYS_getuid), _ret == -1 && errno == EACCES);
	CHECK_WITH(pthread_barrier_wait(&observer_barrier),
		   _ret == 0 || _ret == PTHREAD_BARRIER_SERIAL_THREAD);
	CHECK(pthread_join(observer, NULL));
	CHECK_WITH(write(release_pipe[1], &byte, 1), _ret == 1);
	CHECK(pthread_join(strict, NULL));
	CHECK_WITH(atomic_load(&escaped), _ret == 0);
	/* A dead strict thread no longer prevents TSYNC. */
	CHECK_WITH(sync_filter(SECCOMP_FILTER_FLAG_TSYNC), _ret == 0);
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
	close_pipes();
	CHECK(pthread_barrier_destroy(&observer_barrier));
}

static volatile sig_atomic_t signal_seen;
static void handler(int signal)
{
	if (signal != SIGUSR1)
		finish(1);
	signal_seen = 1;
}

static void signal_return_and_mask(void)
{
	setup_pipes();
	pid_t pid = CHECK(fork());
	if (pid == 0) {
		struct sigaction action = { .sa_handler = handler,
					    .sa_flags = SA_RESTART };
		sigemptyset(&action.sa_mask);
		CHECK(sigaction(SIGUSR1, &action, NULL));
		CHECK(sigaction(SIGUSR2, &action, NULL));
		sigset_t blocked;
		sigemptyset(&blocked);
		sigaddset(&blocked, SIGUSR2);
		CHECK(sigprocmask(SIG_BLOCK, &blocked, NULL));
		enter_strict();
		char byte = 'r';
		require(syscall(SYS_write, ready_pipe[1], &byte, 1) == 1);
		/* Parent releases read only after queuing both signals. */
		require(syscall(SYS_read, release_pipe[0], &byte, 1) == 1);
		while (!signal_seen)
			;
		finish(0);
	}
	char byte;
	CHECK_WITH(read(ready_pipe[0], &byte, 1), _ret == 1);
	CHECK(kill(pid, SIGUSR2));
	CHECK(kill(pid, SIGUSR1));
	CHECK_WITH(write(release_pipe[1], &byte, 1), _ret == 1);
	int status;
	CHECK_WITH(waitpid(pid, &status, 0), _ret == pid && WIFEXITED(status) &&
						     WEXITSTATUS(status) == 0);
	close_pipes();
}

static void blocked_signals_cannot_prevent_sigkill(void)
{
	setup_pipes();
	pid_t pid = CHECK(fork());
	if (pid == 0) {
		sigset_t blocked;
		sigfillset(&blocked);
		CHECK(sigprocmask(SIG_SETMASK, &blocked, NULL));
		enter_strict();
		char byte = 'r';
		require(syscall(SYS_write, ready_pipe[1], &byte, 1) == 1);
		for (;;)
			;
	}
	char byte;
	CHECK_WITH(read(ready_pipe[0], &byte, 1), _ret == 1);
	CHECK(kill(pid, SIGKILL));
	int status;
	CHECK_WITH(waitpid(pid, &status, 0),
		   _ret == pid && WIFSIGNALED(status) &&
			   WTERMSIG(status) == SIGKILL);
	close_pipes();
}

static atomic_int race_ready, race_go, strict_result;
static void *racing_target(void *unused)
{
	(void)unused;
	strict_tid = syscall(SYS_gettid);
	atomic_store(&race_ready, 1);
	while (!atomic_load(&race_go))
		;
	int result = syscall(SYS_seccomp, SECCOMP_SET_MODE_STRICT, 0, NULL);
	int error = errno;
	require(result == 0 || (result == -1 && error == EINVAL));
	atomic_store(&strict_result, result == 0 ? 1 : 2);
	while (atomic_load(&race_go) != 2)
		;
	finish(0);
}

static void strict_sync_race(void)
{
	pthread_t thread;
	CHECK(pthread_create(&thread, NULL, racing_target, NULL));
	while (!atomic_load(&race_ready))
		sched_yield();
	no_new_privs();
	atomic_store(&race_go, 1);
	int result = sync_filter(SECCOMP_FILTER_FLAG_TSYNC);
	CHECK_WITH(result, _ret == 0 || _ret == strict_tid);
	while (!atomic_load(&strict_result))
		sched_yield();
	CHECK_WITH(atomic_load(&strict_result), _ret == (result == 0 ? 2 : 1));
	if (result == 0) {
		CHECK_WITH(prctl(PR_GET_SECCOMP), _ret == SECCOMP_MODE_FILTER);
		CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
	} else {
		CHECK_WITH(prctl(PR_GET_SECCOMP),
			   _ret == SECCOMP_MODE_DISABLED);
		CHECK(syscall(SYS_getppid));
	}
	atomic_store(&race_go, 2);
	CHECK(pthread_join(thread, NULL));
}

#ifdef __x86_64__
static int use_rdtscp;
static void timestamp_instruction(void)
{
	unsigned int low, high, aux;
	if (use_rdtscp)
		__asm__ volatile("rdtscp" : "=a"(low), "=d"(high), "=c"(aux));
	else
		__asm__ volatile("rdtsc" : "=a"(low), "=d"(high));
}

static void strict_timestamp(void)
{
	enter_strict();
	timestamp_instruction();
	finish(1);
}

static void timestamp_still_enabled(void)
{
	for (int i = 0; i < 32; ++i) {
		timestamp_instruction();
		sched_yield();
	}
}
#endif

#ifdef __x86_64__
static void timestamp_after_signal_return(void)
{
	setup_pipes();
	pid_t pid = CHECK(fork());
	if (pid == 0) {
		struct sigaction action = { .sa_handler = handler };
		sigemptyset(&action.sa_mask);
		CHECK(sigaction(SIGUSR1, &action, NULL));
		enter_strict();
		char byte = 'r';
		require(syscall(SYS_write, ready_pipe[1], &byte, 1) == 1);
		while (!signal_seen)
			;
		timestamp_instruction();
		finish(1);
	}
	char byte;
	CHECK_WITH(read(ready_pipe[0], &byte, 1), _ret == 1);
	CHECK(kill(pid, SIGUSR1));
	int status;
	CHECK_WITH(waitpid(pid, &status, 0),
		   _ret == pid && WIFSIGNALED(status) &&
			   WTERMSIG(status) == SIGSEGV);
	close_pipes();
}
#endif

FN_TEST(strict)
{
	alarm(120);
	TEST_RES(run_child(invalid_arguments, 0), _ret == 0);
	TEST_RES(run_child(filter_cannot_become_strict, 0), _ret == 0);
	for (use_prctl = 0; use_prctl < 2; ++use_prctl)
		for (set_nnp = 0; set_nnp < 2; ++set_nnp)
			TEST_RES(run_child(unprivileged_io, 0), _ret == 0);
	int forbidden[] = {
		SYS_getpid, SYS_exit_group, SYS_prctl,		SYS_seccomp,
		SYS_clone,  SYS_execve,	    SYS_rt_sigprocmask, -1,
#ifdef SYS_fork
		SYS_fork,
#endif
	};
	for (size_t i = 0; i < sizeof(forbidden) / sizeof(forbidden[0]); ++i) {
		forbidden_nr = forbidden[i];
		TEST_RES(run_child(forbidden_call, SIGKILL), _ret == 0);
	}
	TEST_RES(run_child(signal_return_and_mask, 0), _ret == 0);
	TEST_RES(run_child(blocked_signals_cannot_prevent_sigkill, 0),
		 _ret == 0);
	for (caller_has_filter = 0; caller_has_filter < 2; ++caller_has_filter)
		TEST_RES(run_child(strict_blocks_sync, 0), _ret == 0);
	for (int i = 0; i < 16; ++i)
		TEST_RES(run_child(strict_sync_race, 0), _ret == 0);
#ifdef __x86_64__
	/* Force restricted and unrestricted contexts onto the same CPU. */
	cpu_set_t available, selected;
	CHECK(sched_getaffinity(0, sizeof(available), &available));
	CPU_ZERO(&selected);
	for (int cpu = 0; cpu < CPU_SETSIZE; ++cpu) {
		if (CPU_ISSET(cpu, &available)) {
			CPU_SET(cpu, &selected);
			break;
		}
	}
	CHECK(sched_setaffinity(0, sizeof(selected), &selected));
	for (use_rdtscp = 0; use_rdtscp < 2; ++use_rdtscp) {
		TEST_RES(run_child(strict_timestamp, SIGSEGV), _ret == 0);
		TEST_RES(run_child(timestamp_after_signal_return, 0),
			 _ret == 0);
		TEST_RES(run_child(timestamp_still_enabled, 0), _ret == 0);
	}
	CHECK(sched_setaffinity(0, sizeof(available), &available));
#endif
	alarm(0);
}
END_TEST()

#ifdef __x86_64__
static void probe_trampolines(void)
{
	enter_strict();
	/* A kernel without uprobes dispatches these allowed calls to ENOSYS. */
	require(syscall(335) == -1 && errno == ENOSYS);
	require(syscall(336) == -1 && errno == ENOSYS);
	finish(0);
}

FN_TEST(strict_probe_trampolines)
{
	/* Linux 6.8 predates these entries. New Linux kernels with uprobes can
	 * reject non-trampoline IPs with SIGILL instead of returning ENOSYS.
	 * Enable this extra check for a kernel with the ABI but no probe backend. */
	SKIP_TEST_IF(!getenv("SECCOMP_TEST_PROBE_STUBS"));
	TEST_RES(run_child(probe_trampolines, 0), _ret == 0);
}
END_TEST()
#endif
