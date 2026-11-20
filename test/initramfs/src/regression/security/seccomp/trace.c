// SPDX-License-Identifier: MPL-2.0

#include "seccomp_common.h"
#include <sys/ptrace.h>
#if defined(__x86_64__)
#include <sys/user.h>
#endif

static void trace_without_tracer(void)
{
	no_new_privs();
	deny_syscall(SYS_getppid, SECCOMP_RET_TRACE | 0xffff);
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == ENOSYS);
}

static int trace_without_option(void)
{
	pid_t child = CHECK(fork());
	if (!child) {
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
		CHECK(raise(SIGSTOP));
		trace_without_tracer();
		_exit(0);
	}
	int status;
	CHECK_WITH(waitpid(child, &status, 0),
		   _ret == child && WIFSTOPPED(status));
	CHECK(ptrace(PTRACE_CONT, child, 0, 0));
	CHECK_WITH(waitpid(child, &status, 0), _ret == child);
	return !WIFEXITED(status) || WEXITSTATUS(status) != 0;
}

#if defined(__x86_64__)
enum trace_case {
	CONTINUE_UNCHANGED,
	REWRITE_ALLOWED,
	REWRITE_DENIED,
	REWRITE_ARGUMENT,
	SKIP_MINUS_ONE,
	SKIP_MINUS_512,
	REWRITE_TRACE,
	DETACH,
	DISABLE_OPTION,
	SYSCALL_STOPS,
	ERRNO_PRECEDENCE,
};

static void install_trace_filters(enum trace_case which)
{
	no_new_privs();
	deny_syscall(SYS_getppid, SECCOMP_RET_TRACE | 5);
	/* Equal precedence selects the newest filter's data. */
	deny_syscall(SYS_getppid, SECCOMP_RET_TRACE | 0xbeef);
	if (which == REWRITE_DENIED)
		deny_syscall(SYS_getpid, SECCOMP_RET_ERRNO | EBUSY);
	if (which == REWRITE_TRACE)
		deny_syscall(SYS_getpid, SECCOMP_RET_TRACE | 0xabcd);
	if (which == ERRNO_PRECEDENCE)
		deny_syscall(SYS_getppid, SECCOMP_RET_ERRNO | EBUSY);
	if (which == SKIP_MINUS_ONE || which == SKIP_MINUS_512)
		deny_syscall(which == SKIP_MINUS_ONE ? -1 : -512,
			     SECCOMP_RET_KILL_PROCESS);
	if (which == REWRITE_ARGUMENT) {
		struct sock_filter code[] = {
			BPF_STMT(BPF_LD | BPF_W | BPF_ABS,
				 offsetof(struct seccomp_data, nr)),
			BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_getppid, 0, 3),
			BPF_STMT(BPF_LD | BPF_W | BPF_ABS,
				 offsetof(struct seccomp_data, args[0])),
			BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, 42, 0, 1),
			BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ERRNO | EACCES),
			BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
		};
		CHECK(install(code, sizeof(code) / sizeof(code[0])));
	}
}

static int trace_event(enum trace_case which)
{
	fflush(NULL);
	pid_t child = CHECK(fork());
	if (!child) {
		long pid = getpid();
		long parent = getppid();
		install_trace_filters(which);
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
		/* raise() may call getpid(), which this filter can reject. */
		CHECK(syscall(SYS_tgkill, pid, pid, SIGSTOP));
		long result = syscall(SYS_getppid, 0);
		int error = errno;
		if (which == REWRITE_DENIED || which == ERRNO_PRECEDENCE)
			_exit(!(result == -1 && error == EBUSY));
		if (which == REWRITE_ARGUMENT)
			_exit(!(result == -1 && error == EACCES));
		if (which == REWRITE_ALLOWED || which == REWRITE_TRACE)
			_exit(result != pid);
		if (which == SKIP_MINUS_ONE || which == SKIP_MINUS_512)
			_exit(result != 1234);
		if (result != parent)
			_exit(91);
		if (which == DETACH || which == DISABLE_OPTION) {
			result = syscall(SYS_getppid, 0);
			_exit(!(result == -1 && errno == ENOSYS));
		}
		_exit(0);
	}

	int status;
	CHECK_WITH(waitpid(child, &status, 0),
		   _ret == child && WIFSTOPPED(status));
	CHECK(ptrace(PTRACE_SETOPTIONS, child, 0,
		     PTRACE_O_TRACESECCOMP | PTRACE_O_TRACESYSGOOD));
	int request = which == SYSCALL_STOPS ? PTRACE_SYSCALL : PTRACE_CONT;
	int events = 0;
	int syscall_stops = 0;
	for (;;) {
		CHECK(ptrace(request, child, 0, 0));
		CHECK_WITH(waitpid(child, &status, 0), _ret == child);
		if (WIFEXITED(status) || WIFSIGNALED(status))
			break;
		if (which == SYSCALL_STOPS &&
		    WSTOPSIG(status) == (SIGTRAP | 0x80)) {
			/* entry -> seccomp event -> exit; then stop syscall tracing. */
			CHECK_WITH(events, _ret == syscall_stops);
			struct user_regs_struct regs;
			CHECK(ptrace(PTRACE_GETREGS, child, 0, &regs));
			CHECK_WITH(regs.orig_rax, _ret == SYS_getppid);
			if (!syscall_stops)
				CHECK_WITH((long)regs.rax, _ret == -ENOSYS);
			if (++syscall_stops == 2)
				request = PTRACE_CONT;
			continue;
		}
		CHECK_WITH(status >> 8,
			   _ret == (SIGTRAP | (PTRACE_EVENT_SECCOMP << 8)));
		CHECK_WITH(++events, _ret == 1);
		unsigned long message;
		CHECK(ptrace(PTRACE_GETEVENTMSG, child, 0, &message));
		CHECK_WITH(message, _ret == 0xbeef);
		siginfo_t info;
		CHECK(ptrace(PTRACE_GETSIGINFO, child, 0, &info));
		CHECK_WITH(info.si_code,
			   _ret == (SIGTRAP | (PTRACE_EVENT_SECCOMP << 8)));
		struct user_regs_struct regs;
		CHECK(ptrace(PTRACE_GETREGS, child, 0, &regs));
		CHECK_WITH(regs.orig_rax, _ret == SYS_getppid);
		switch (which) {
		case REWRITE_ALLOWED:
		case REWRITE_DENIED:
		case REWRITE_TRACE:
			regs.orig_rax = SYS_getpid;
			break;
		case REWRITE_ARGUMENT:
			regs.rdi = 42;
			break;
		case SKIP_MINUS_ONE:
		case SKIP_MINUS_512:
			regs.orig_rax = which == SKIP_MINUS_ONE ? -1UL : -512UL;
			regs.rax = 1234;
			break;
		default:
			break;
		}
		CHECK(ptrace(PTRACE_SETREGS, child, 0, &regs));
		if (which == DISABLE_OPTION)
			CHECK(ptrace(PTRACE_SETOPTIONS, child, 0, 0));
		if (which == DETACH) {
			CHECK(ptrace(PTRACE_DETACH, child, 0, 0));
			CHECK_WITH(waitpid(child, &status, 0), _ret == child);
			break;
		}
	}
	return !WIFEXITED(status) || WEXITSTATUS(status) != 0 ||
	       events != (which != ERRNO_PRECEDENCE) ||
	       (which == SYSCALL_STOPS && syscall_stops != 2);
}
#endif

static int kill_during_event(void)
{
	int pipefd[2];
	CHECK(pipe(pipefd));
	pid_t child = CHECK(fork());
	if (!child) {
		CHECK(close(pipefd[0]));
		no_new_privs();
		deny_syscall(SYS_write, SECCOMP_RET_TRACE);
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
		CHECK(raise(SIGSTOP));
		syscall(SYS_write, pipefd[1], "x", 1);
		_exit(90);
	}
	CHECK(close(pipefd[1]));
	int status;
	CHECK_WITH(waitpid(child, &status, 0),
		   _ret == child && WIFSTOPPED(status));
	CHECK(ptrace(PTRACE_SETOPTIONS, child, 0, PTRACE_O_TRACESECCOMP));
	CHECK(ptrace(PTRACE_CONT, child, 0, 0));
	CHECK_WITH(waitpid(child, &status, 0),
		   _ret == child && WIFSTOPPED(status) &&
			   (status >> 16) == PTRACE_EVENT_SECCOMP);
	CHECK(kill(child, SIGKILL));
	CHECK_WITH(waitpid(child, &status, 0), _ret == child);
	char byte;
	int count = CHECK(read(pipefd[0], &byte, 1));
	CHECK(close(pipefd[0]));
	return !WIFSIGNALED(status) || WTERMSIG(status) != SIGKILL || count;
}

FN_TEST(seccomp_trace)
{
	unsigned int action = SECCOMP_RET_TRACE;
	TEST_SUCC(syscall(SYS_seccomp, SECCOMP_GET_ACTION_AVAIL, 0, &action));
	TEST_RES(run_child(trace_without_tracer, 0), _ret == 0);
	TEST_RES(trace_without_option(), _ret == 0);
#if defined(__x86_64__)
	TEST_RES(trace_event(CONTINUE_UNCHANGED), _ret == 0);
	TEST_RES(trace_event(REWRITE_ALLOWED), _ret == 0);
	TEST_RES(trace_event(REWRITE_DENIED), _ret == 0);
	TEST_RES(trace_event(REWRITE_ARGUMENT), _ret == 0);
	TEST_RES(trace_event(SKIP_MINUS_ONE), _ret == 0);
	TEST_RES(trace_event(SKIP_MINUS_512), _ret == 0);
	TEST_RES(trace_event(REWRITE_TRACE), _ret == 0);
	TEST_RES(trace_event(DISABLE_OPTION), _ret == 0);
	TEST_RES(trace_event(SYSCALL_STOPS), _ret == 0);
	TEST_RES(trace_event(ERRNO_PRECEDENCE), _ret == 0);
#endif
	TEST_RES(kill_during_event(), _ret == 0);
}
END_TEST()

#if defined(__x86_64__)
FN_TEST(seccomp_trace_detach)
{
	/* An unrecognized ptrace request returns EIO on Asterinas. */
	long result = ptrace(PTRACE_DETACH, getpid(), 0, 0);
	int error = errno;
#ifdef __asterinas__
	SKIP_TEST_IF(result == -1 && error == EIO);
#endif
	TEST_RES(result, _ret == -1 && error == ESRCH);
	TEST_RES(trace_event(DETACH), _ret == 0);
}
END_TEST()
#endif
