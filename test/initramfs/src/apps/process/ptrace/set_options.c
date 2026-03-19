// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

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