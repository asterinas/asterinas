// SPDX-License-Identifier: MPL-2.0

#include "seccomp_common.h"
#include "seccomp_log_capture.h"
#include <sys/ptrace.h>
#if defined(__x86_64__)
#include <sys/user.h>
#endif

static int install_action(uint32_t action, unsigned int flags)
{
	struct sock_filter code[] = {
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS,
			 offsetof(struct seccomp_data, nr)),
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_getppid, 0, 1),
		BPF_STMT(BPF_RET | BPF_K, action),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
	};
	struct sock_fprog prog = { .len = 4, .filter = code };
	return syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER, flags, &prog);
}

/* A host-side checker compares these expectations with actual kernel records.
 * A successful syscall alone does not demonstrate logging support.
 */
static void expect_syscall_log(int nr, uint32_t action, int count)
{
	printf("LOG_EXPECT pid=%d syscall=%d action=%#x count=%d\n", getpid(),
	       nr, action, count);
	fflush(stdout);
}

static void expect_log(uint32_t action, int count)
{
	expect_syscall_log(SYS_getppid, action, count);
}

static void log_allows(void)
{
	long parent = getppid();
	no_new_privs();
	expect_log(SECCOMP_RET_LOG, 1);
	CHECK(install_action(SECCOMP_RET_LOG | 0xffff, 0));
	CHECK_WITH(syscall(SYS_getppid), _ret == parent);
}

static void allow_with_flag_is_silent(void)
{
	long parent = getppid();
	no_new_privs();
	expect_log(SECCOMP_RET_ALLOW, 0);
	CHECK(install_action(SECCOMP_RET_ALLOW, SECCOMP_FILTER_FLAG_LOG));
	CHECK_WITH(syscall(SYS_getppid), _ret == parent);
}

static void errno_with_flag(void)
{
	no_new_privs();
	expect_log(SECCOMP_RET_ERRNO, 1);
	CHECK(install_action(SECCOMP_RET_ERRNO | EBUSY,
			     SECCOMP_FILTER_FLAG_LOG));
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
}

static void newest_silent_wins(void)
{
	no_new_privs();
	expect_log(SECCOMP_RET_ERRNO, 0);
	CHECK(install_action(SECCOMP_RET_ERRNO | EBUSY,
			     SECCOMP_FILTER_FLAG_LOG));
	CHECK(install_action(SECCOMP_RET_ERRNO | EACCES, 0));
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EACCES);
}

static void newest_logging_wins(void)
{
	no_new_privs();
	expect_log(SECCOMP_RET_ERRNO, 1);
	CHECK(install_action(SECCOMP_RET_ERRNO | EACCES, 0));
	CHECK(install_action(SECCOMP_RET_ERRNO | EBUSY,
			     SECCOMP_FILTER_FLAG_LOG));
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
}

static void losing_log_is_silent(void)
{
	no_new_privs();
	expect_log(SECCOMP_RET_ERRNO, 0);
	CHECK(install_action(SECCOMP_RET_LOG, SECCOMP_FILTER_FLAG_LOG));
	CHECK(install_action(SECCOMP_RET_ERRNO | EBUSY, 0));
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
}

static void older_errno_wins(void)
{
	no_new_privs();
	expect_log(SECCOMP_RET_ERRNO, 1);
	CHECK(install_action(SECCOMP_RET_ERRNO | EBUSY,
			     SECCOMP_FILTER_FLAG_LOG));
	CHECK(install_action(SECCOMP_RET_LOG, 0));
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
}

static void trace_without_tracer_logs(void)
{
	no_new_privs();
	expect_log(SECCOMP_RET_TRACE, 1);
	CHECK(install_action(SECCOMP_RET_TRACE | 42, SECCOMP_FILTER_FLAG_LOG));
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == ENOSYS);
}

static void trap_logs(void)
{
	no_new_privs();
	expect_log(SECCOMP_RET_TRAP, 1);
	CHECK(install_action(SECCOMP_RET_TRAP, SECCOMP_FILTER_FLAG_LOG));
	syscall(SYS_getppid);
	_exit(1);
}

static void kill_logs(void)
{
	no_new_privs();
	expect_log(SECCOMP_RET_KILL_PROCESS, 1);
	CHECK(install_action(SECCOMP_RET_KILL_PROCESS, 0));
	syscall(SYS_getppid);
	_exit(1);
}

static void unknown_logs_raw_action(void)
{
	no_new_privs();
	expect_log(0x12340000, 1);
	CHECK(install_action(0x12340000, 0));
	syscall(SYS_getppid);
	_exit(1);
}

static void invalid_flags_preserve_logging(void)
{
	no_new_privs();
	expect_log(SECCOMP_RET_ERRNO, 1);
	CHECK(install_action(SECCOMP_RET_ERRNO | EBUSY,
			     SECCOMP_FILTER_FLAG_LOG));
	CHECK_WITH(install_action(SECCOMP_RET_ALLOW,
				  0x80000000 | SECCOMP_FILTER_FLAG_LOG),
		   _ret == -1 && errno == EINVAL);
	CHECK_WITH(syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER,
			   0x80000000 | SECCOMP_FILTER_FLAG_LOG, NULL),
		   _ret == -1 && errno == EINVAL);
	CHECK_WITH(syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER,
			   SECCOMP_FILTER_FLAG_LOG, NULL),
		   _ret == -1 && errno == EFAULT);
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
}

static void log_flag_combinations(void)
{
	no_new_privs();
	expect_log(SECCOMP_RET_ERRNO, 3);
	unsigned int flags[] = {
		SECCOMP_FILTER_FLAG_LOG | SECCOMP_FILTER_FLAG_TSYNC,
		SECCOMP_FILTER_FLAG_LOG | SECCOMP_FILTER_FLAG_TSYNC_ESRCH,
		SECCOMP_FILTER_FLAG_LOG | SECCOMP_FILTER_FLAG_TSYNC |
			SECCOMP_FILTER_FLAG_TSYNC_ESRCH,
	};
	for (size_t i = 0; i < sizeof(flags) / sizeof(flags[0]); ++i) {
		CHECK(install_action(SECCOMP_RET_ERRNO | EBUSY, flags[i]));
		CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
	}
}

static pthread_barrier_t barrier;

static void rendezvous(void)
{
	CHECK_WITH(pthread_barrier_wait(&barrier),
		   _ret == 0 || _ret == PTHREAD_BARRIER_SERIAL_THREAD);
}

static void *synced_thread(void *arg)
{
	(void)arg;
	rendezvous();
	rendezvous();
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
	return NULL;
}

static void tsync_propagates_logging(void)
{
	no_new_privs();
	expect_log(SECCOMP_RET_ERRNO, 1);
	CHECK(pthread_barrier_init(&barrier, NULL, 2));
	pthread_t thread;
	CHECK(pthread_create(&thread, NULL, synced_thread, NULL));
	rendezvous();
	CHECK(install_action(SECCOMP_RET_ERRNO | EBUSY,
			     SECCOMP_FILTER_FLAG_LOG |
				     SECCOMP_FILTER_FLAG_TSYNC));
	rendezvous();
	CHECK(pthread_join(thread, NULL));
	CHECK(pthread_barrier_destroy(&barrier));
}

static long divergent_tid;

static void *divergent_thread(void *arg)
{
	(void)arg;
	divergent_tid = syscall(SYS_gettid);
	CHECK(install_action(SECCOMP_RET_ERRNO | EACCES, 0));
	rendezvous();
	rendezvous();
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EACCES);
	return NULL;
}

static void failed_tsync_preserves_logging(void)
{
	no_new_privs();
	expect_log(SECCOMP_RET_ERRNO, 1);
	CHECK(pthread_barrier_init(&barrier, NULL, 2));
	pthread_t thread;
	CHECK(pthread_create(&thread, NULL, divergent_thread, NULL));
	rendezvous();
	CHECK(install_action(SECCOMP_RET_ERRNO | EBUSY,
			     SECCOMP_FILTER_FLAG_LOG));
	CHECK_WITH(install_action(SECCOMP_RET_ALLOW,
				  SECCOMP_FILTER_FLAG_LOG |
					  SECCOMP_FILTER_FLAG_TSYNC),
		   _ret == divergent_tid);
	CHECK_WITH(install_action(SECCOMP_RET_ALLOW,
				  SECCOMP_FILTER_FLAG_LOG |
					  SECCOMP_FILTER_FLAG_TSYNC |
					  SECCOMP_FILTER_FLAG_TSYNC_ESRCH),
		   _ret == -1 && errno == ESRCH);
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
	rendezvous();
	CHECK(pthread_join(thread, NULL));
	CHECK(pthread_barrier_destroy(&barrier));
}

static void inherited_logging_call(void)
{
	expect_log(SECCOMP_RET_ERRNO, 1);
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EBUSY);
}

static void fork_inherits_logging(void)
{
	no_new_privs();
	CHECK(install_action(SECCOMP_RET_ERRNO | EBUSY,
			     SECCOMP_FILTER_FLAG_LOG));
	CHECK_WITH(run_child(inherited_logging_call, 0), _ret == 0);
}

#if defined(__x86_64__)
static int trace_log(int skip)
{
	fflush(NULL);
	pid_t child = CHECK(fork());
	if (!child) {
		long parent = getppid();
		no_new_privs();
		expect_log(SECCOMP_RET_TRACE, 0);
		if (skip)
			expect_syscall_log(-1, SECCOMP_RET_TRACE, 1);
		CHECK(install_action(SECCOMP_RET_TRACE,
				     SECCOMP_FILTER_FLAG_LOG));
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
		CHECK(raise(SIGSTOP));
		long result = syscall(SYS_getppid);
		_exit(result != (skip ? 1234 : parent));
	}
	int status;
	CHECK_WITH(waitpid(child, &status, 0),
		   _ret == child && WIFSTOPPED(status));
	CHECK(ptrace(PTRACE_SETOPTIONS, child, 0, PTRACE_O_TRACESECCOMP));
	CHECK(ptrace(PTRACE_CONT, child, 0, 0));
	CHECK_WITH(waitpid(child, &status, 0),
		   _ret == child && WIFSTOPPED(status) &&
			   (unsigned int)status >> 16 == PTRACE_EVENT_SECCOMP);
	if (skip) {
		struct user_regs_struct regs;
		CHECK(ptrace(PTRACE_GETREGS, child, 0, &regs));
		regs.orig_rax = -1;
		regs.rax = 1234;
		CHECK(ptrace(PTRACE_SETREGS, child, 0, &regs));
	}
	CHECK(ptrace(PTRACE_CONT, child, 0, 0));
	CHECK_WITH(waitpid(child, &status, 0), _ret == child);
	return !(WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
#endif

FN_TEST(logging)
{
	struct log_capture capture = capture_log_output();

	uint32_t action = SECCOMP_RET_LOG;
	TEST_SUCC(syscall(SYS_seccomp, SECCOMP_GET_ACTION_AVAIL, 0, &action));
	action |= 1;
	TEST_ERRNO(syscall(SYS_seccomp, SECCOMP_GET_ACTION_AVAIL, 0, &action),
		   EOPNOTSUPP);
	TEST_RES(run_child(log_allows, 0), _ret == 0);
	TEST_RES(run_child(allow_with_flag_is_silent, 0), _ret == 0);
	TEST_RES(run_child(errno_with_flag, 0), _ret == 0);
	TEST_RES(run_child(newest_silent_wins, 0), _ret == 0);
	TEST_RES(run_child(newest_logging_wins, 0), _ret == 0);
	TEST_RES(run_child(losing_log_is_silent, 0), _ret == 0);
	TEST_RES(run_child(older_errno_wins, 0), _ret == 0);
	TEST_RES(run_child(trace_without_tracer_logs, 0), _ret == 0);
	TEST_RES(run_child(trap_logs, SIGSYS), _ret == 0);
	TEST_RES(run_child(kill_logs, SIGSYS), _ret == 0);
	TEST_RES(run_child(unknown_logs_raw_action, SIGSYS), _ret == 0);
	TEST_RES(run_child(invalid_flags_preserve_logging, 0), _ret == 0);
	TEST_RES(run_child(log_flag_combinations, 0), _ret == 0);
	TEST_RES(run_child(tsync_propagates_logging, 0), _ret == 0);
	TEST_RES(run_child(failed_tsync_preserves_logging, 0), _ret == 0);
	TEST_RES(run_child(fork_inherits_logging, 0), _ret == 0);
#if defined(__x86_64__)
	TEST_RES(trace_log(0), _ret == 0);
	TEST_RES(trace_log(1), _ret == 0);
#endif
	flush_log_output(capture);
}
END_TEST()
