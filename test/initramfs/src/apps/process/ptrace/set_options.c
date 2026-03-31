// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <sched.h>
#include <signal.h>
#include <sys/ptrace.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"
#include "../../common/yama_ptrace_scope.h"

static int ptrace_event_status(int event)
{
	return SIGTRAP | (event << 8);
}

#define PTRACE_SETOPTIONS_TEST(name, tracee_fn)                             \
	SKIP_TEST_IF(read_yama_scope() == YAMA_SCOPE_NO_ATTACH);            \
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

#define CLEANUP_GRAND_CHILD(notify_sig)                             \
	pid_t grand_child = eventmsg;                               \
	TEST_RES(waitpid(grand_child, &status, 0),                  \
		 _ret == grand_child && WIFSTOPPED(status) &&       \
			 WSTOPSIG(status) == SIGSTOP);              \
	TEST_SUCC(ptrace(PTRACE_CONT, grand_child, 0, 0));          \
	TEST_RES(waitpid(grand_child, &status, 0),                  \
		 _ret == grand_child && WIFEXITED(status) &&        \
			 WEXITSTATUS(status) == 0);                 \
	if (notify_sig) {                                           \
		TEST_SUCC(ptrace(PTRACE_CONT, pid, 0, 0));          \
		TEST_RES(waitpid(pid, &status, 0),                  \
			 _ret == pid && WIFSTOPPED(status) &&       \
				 WSTOPSIG(status) == (notify_sig)); \
	}

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

static int clone_child_fn(void *arg)
{
	return 0;
}

#define STACK_SIZE (1 << 20)
static void tracee_clone_process(void)
{
	static char child_stack[STACK_SIZE];
	CHECK(clone(clone_child_fn, child_stack + STACK_SIZE, SIGUSR1, NULL));
	_exit(0);
}

static void tracee_clone_thread(void)
{
	static char child_stack[STACK_SIZE];
	CHECK(clone(clone_child_fn, child_stack + STACK_SIZE,
		    CLONE_PARENT | CLONE_THREAD | CLONE_VM | CLONE_SIGHAND,
		    NULL));
	_exit(0);
}
#undef STACK_SIZE

FN_TEST(ptrace_trace_clone_process)
{
	PTRACE_SETOPTIONS_TEST(CLONE, tracee_clone_process);

	CLEANUP_GRAND_CHILD(SIGUSR1);
	CLEANUP_CHILD();
}
END_TEST()

FN_TEST(ptrace_trace_clone_thread)
{
	PTRACE_SETOPTIONS_TEST(CLONE, tracee_clone_thread);

	CLEANUP_GRAND_CHILD(0);
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

FN_TEST(ptrace_trace_vfork_both)
{
#define PTRACE_O_TRACEVFORK_BOTH (PTRACE_O_TRACEVFORK | PTRACE_O_TRACEVFORKDONE)
// The first ptrace-event-stop is PTRACE_EVENT_VFORK.
#define PTRACE_EVENT_VFORK_BOTH PTRACE_EVENT_VFORK
	PTRACE_SETOPTIONS_TEST(VFORK_BOTH, tracee_vfork);
#undef PTRACE_O_TRACEVFORK_BOTH
#undef PTRACE_EVENT_VFORK_BOTH

	pid_t grand_child = eventmsg;
	TEST_RES(waitpid(grand_child, &status, 0),
		 _ret == grand_child && WIFSTOPPED(status) &&
			 WSTOPSIG(status) == SIGSTOP);
	TEST_SUCC(ptrace(PTRACE_CONT, grand_child, 0, 0));
	TEST_RES(waitpid(grand_child, &status, 0),
		 _ret == grand_child && WIFEXITED(status) &&
			 WEXITSTATUS(status) == 0);

	TEST_SUCC(ptrace(PTRACE_CONT, pid, 0, 0));
	int vfork_done_status = ptrace_event_status(PTRACE_EVENT_VFORK_DONE);
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFSTOPPED(status) &&
			 (status >> 8) == vfork_done_status);
	TEST_RES(ptrace(PTRACE_GETSIGINFO, pid, 0, &si),
		 si.si_signo == SIGTRAP && si.si_code == vfork_done_status &&
			 si.si_pid == pid && si.si_uid == getuid());
	TEST_RES(ptrace(PTRACE_GETEVENTMSG, pid, 0, &eventmsg),
		 eventmsg == grand_child);

	TEST_SUCC(ptrace(PTRACE_CONT, pid, 0, 0));
	TEST_RES(waitpid(pid, &status, 0), _ret == pid && WIFSTOPPED(status) &&
						   WSTOPSIG(status) == SIGCHLD);

	CLEANUP_CHILD();
}
END_TEST()

#endif