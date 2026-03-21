// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <sched.h>
#include <signal.h>
#include <sys/ptrace.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

static int ptrace_event_status(int event)
{
	return SIGTRAP | (event << 8);
}

#define PTRACE_SETOPTIONS_TEST(name, tracee_fn)                             \
	pid_t pid = TEST_SUCC(fork());                                      \
	if (pid == 0) {                                                     \
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));                     \
		CHECK(raise(SIGSTOP));                                      \
		tracee_fn();                                                \
	}                                                                   \
                                                                            \
	int status;                                                         \
	unsigned long eventmsg;                                             \
	siginfo_t si;                                                       \
	TEST_RES(waitpid(pid, &status, 0),                                  \
		 _ret == pid && WIFSTOPPED(status) &&                       \
			 WSTOPSIG(status) == SIGSTOP);                      \
	TEST_SUCC(ptrace(PTRACE_SETOPTIONS, pid, 0, PTRACE_O_TRACE##name)); \
	TEST_SUCC(ptrace(PTRACE_CONT, pid, 0, 0));                          \
                                                                            \
	int event_status = ptrace_event_status(PTRACE_EVENT_##name);        \
	TEST_RES(waitpid(pid, &status, 0),                                  \
		 _ret == pid && WIFSTOPPED(status) &&                       \
			 (status >> 8) == event_status);                    \
	TEST_RES(ptrace(PTRACE_GETSIGINFO, pid, 0, &si),                    \
		 si.si_signo == SIGTRAP && si.si_code == event_status &&    \
			 si.si_pid == pid && si.si_uid == getuid());        \
	TEST_SUCC(ptrace(PTRACE_GETEVENTMSG, pid, 0, &eventmsg));

#define CLEANUP_CHILD()                                                        \
	TEST_SUCC(ptrace(PTRACE_CONT, pid, 0, 0));                             \
	TEST_RES(waitpid(pid, &status, 0), _ret == pid && WIFEXITED(status) && \
						   WEXITSTATUS(status) == 0);

static void tracee_exec(void)
{
	char *const argv[] = { "/bin/true", NULL };
	char *const envp[] = { NULL };
	CHECK(execve("/bin/true", argv, envp));
	_exit(-1);
}

FN_TEST(ptrace_trace_exec)
{
	PTRACE_SETOPTIONS_TEST(EXEC, tracee_exec);

	TEST_RES(eventmsg, _ret == pid);
	CLEANUP_CHILD();
}
END_TEST()

static void tracee_exit(void)
{
	_exit(0);
}

FN_TEST(ptrace_trace_exit)
{
	PTRACE_SETOPTIONS_TEST(EXIT, tracee_exit);

	TEST_RES(eventmsg, WIFEXITED(_ret) && WEXITSTATUS(_ret) == 0);
	CLEANUP_CHILD();
}
END_TEST()

// TODO: Support clone-family ptrace events and remove this guard.
#ifndef __asterinas__

#define CLEANUP_GRAND_CHILD(notify_sig)                       \
	pid_t grand_child = eventmsg;                         \
	TEST_RES(waitpid(grand_child, &status, 0),            \
		 _ret == grand_child && WIFSTOPPED(status) && \
			 WSTOPSIG(status) == SIGSTOP);        \
	TEST_SUCC(ptrace(PTRACE_CONT, grand_child, 0, 0));    \
	TEST_RES(waitpid(grand_child, &status, 0),            \
		 _ret == grand_child && WIFEXITED(status) &&  \
			 WEXITSTATUS(status) == 0);           \
                                                              \
	CHECK(ptrace(PTRACE_CONT, pid, 0, 0));                \
	TEST_RES(waitpid(pid, &status, 0),                    \
		 _ret == pid && WIFSTOPPED(status) &&         \
			 WSTOPSIG(status) == (notify_sig));

static void tracee_fork(void)
{
	CHECK(fork());
	_exit(0);
}

FN_TEST(ptrace_trace_fork)
{
	PTRACE_SETOPTIONS_TEST(FORK, tracee_fork);

	CLEANUP_GRAND_CHILD(SIGCHLD);
	CLEANUP_CHILD();
}
END_TEST()

static void tracee_vfork(void)
{
	CHECK(vfork());
	_exit(0);
}

FN_TEST(ptrace_trace_vfork)
{
	PTRACE_SETOPTIONS_TEST(VFORK, tracee_vfork);

	CLEANUP_GRAND_CHILD(SIGCHLD);
	CLEANUP_CHILD();
}
END_TEST()

static int clone_child_fn(void *arg)
{
	_exit(0);
}

static void tracee_clone(void)
{
#define STACK_SIZE (1 << 20)
	static char child_stack[STACK_SIZE];
	CHECK(clone(clone_child_fn, child_stack + STACK_SIZE, SIGUSR1, NULL));
#undef STACK_SIZE
	_exit(0);
}

FN_TEST(ptrace_trace_clone)
{
	PTRACE_SETOPTIONS_TEST(CLONE, tracee_clone);

	CLEANUP_GRAND_CHILD(SIGUSR1);
	CLEANUP_CHILD();
}
END_TEST()

#endif