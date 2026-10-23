// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include "../../common/test.h"
#include <signal.h>
#include <sys/ptrace.h>
#include <sys/syscall.h>
#include <sys/user.h>
#include <sys/wait.h>
#include <unistd.h>

#ifdef __x86_64__
enum restart_case {
	HANDLER_RESTART,
	HANDLER_INTERRUPT,
	NOINTR_HANDLER,
	IGNORED,
	SUPPRESSED,
	NO_SIGNAL,
	NO_SIGNAL_NOINTR,
	BLOCKED,
	EDIT_RESULT,
	EDIT_NUMBER,
	CANCEL_RESTART,
};

static volatile sig_atomic_t handled;

static void handler(int sig)
{
	(void)sig;
	handled++;
}

/* Inject Linux's internal restart result at a real syscall-exit stop. This
 * exercises restart decisions without timing-dependent blocked-I/O setup. */
static int check_restart(enum restart_case test)
{
	pid_t parent = getpid();
	pid_t pid = CHECK(fork());
	if (pid == 0) {
		pid_t self = getpid();
		struct sigaction action = { .sa_handler = handler };
		if (test == HANDLER_RESTART)
			action.sa_flags = SA_RESTART;
		if (test == IGNORED)
			action.sa_handler = SIG_IGN;
		CHECK(sigemptyset(&action.sa_mask));
		CHECK(sigaction(SIGUSR1, &action, NULL));
		if (test == BLOCKED) {
			sigset_t mask;
			CHECK(sigemptyset(&mask));
			CHECK(sigaddset(&mask, SIGUSR1));
			CHECK(sigprocmask(SIG_BLOCK, &mask, NULL));
		}
		CHECK(ptrace(PTRACE_TRACEME, 0, NULL, NULL));
		CHECK(raise(SIGSTOP));
		errno = 0;
		long result = syscall(SYS_getpid);
		int saved_errno = errno;
		if (test == HANDLER_INTERRUPT)
			CHECK_WITH(result, _ret == -1 && saved_errno == EINTR);
		else if (test == EDIT_RESULT)
			CHECK_WITH(result, _ret == 1234);
		else if (test == EDIT_NUMBER)
			CHECK_WITH(result, _ret == parent);
		else if (test == CANCEL_RESTART || test == NO_SIGNAL ||
			 test == BLOCKED)
			CHECK_WITH(result, _ret == -1 && saved_errno == 512);
		else if (test == NO_SIGNAL_NOINTR)
			CHECK_WITH(result, _ret == -1 && saved_errno == 513);
		else
			CHECK_WITH(result, _ret == self);
		int expected = test == HANDLER_RESTART ||
			       test == HANDLER_INTERRUPT ||
			       test == NOINTR_HANDLER;
		CHECK_WITH(handled, _ret == expected);
		_exit(0);
	}
	int status;
	CHECK_WITH(waitpid(pid, &status, 0), _ret == pid && WIFSTOPPED(status));
	CHECK(ptrace(PTRACE_SETOPTIONS, pid, NULL, PTRACE_O_TRACESYSGOOD));
	struct user_regs_struct regs;
	for (;;) {
		CHECK(ptrace(PTRACE_SYSCALL, pid, NULL, NULL));
		CHECK_WITH(waitpid(pid, &status, 0),
			   _ret == pid && WIFSTOPPED(status) &&
				   WSTOPSIG(status) == (SIGTRAP | 0x80));
		CHECK(ptrace(PTRACE_GETREGS, pid, NULL, &regs));
		if (regs.orig_rax == SYS_getpid)
			break;
	}
	/* Advance from the entry stop to the exit stop. */
	CHECK(ptrace(PTRACE_SYSCALL, pid, NULL, NULL));
	CHECK_WITH(waitpid(pid, &status, 0),
		   _ret == pid && WIFSTOPPED(status) &&
			   WSTOPSIG(status) == (SIGTRAP | 0x80));
	CHECK(ptrace(PTRACE_GETREGS, pid, NULL, &regs));
	regs.rax = test == NOINTR_HANDLER || test == NO_SIGNAL_NOINTR ? -513LL :
									-512LL;
	CHECK(ptrace(PTRACE_SETREGS, pid, NULL, &regs));
	if (test != NO_SIGNAL && test != NO_SIGNAL_NOINTR)
		CHECK(kill(pid, SIGUSR1));
	CHECK(ptrace(PTRACE_CONT, pid, NULL, NULL));
	if (test != NO_SIGNAL && test != NO_SIGNAL_NOINTR && test != BLOCKED) {
		CHECK_WITH(waitpid(pid, &status, 0),
			   _ret == pid && WIFSTOPPED(status) &&
				   WSTOPSIG(status) == SIGUSR1);
		CHECK(ptrace(PTRACE_GETREGS, pid, NULL, &regs));
		CHECK_WITH((long)regs.rax,
			   _ret == (test == NOINTR_HANDLER ? -513 : -512));
		if (test == EDIT_RESULT)
			regs.rax = 1234;
		if (test == EDIT_NUMBER)
			regs.orig_rax = SYS_getppid;
		if (test == CANCEL_RESTART)
			regs.orig_rax = -1ULL;
		CHECK(ptrace(PTRACE_SETREGS, pid, NULL, &regs));
		int sig = test == HANDLER_RESTART ||
					  test == HANDLER_INTERRUPT ||
					  test == NOINTR_HANDLER ||
					  test == IGNORED ?
				  SIGUSR1 :
				  0;
		CHECK(ptrace(PTRACE_CONT, pid, NULL, sig));
	}
	CHECK_WITH(waitpid(pid, &status, 0), _ret == pid);
	return !(WIFEXITED(status) && WEXITSTATUS(status) == 0);
}

static int blocked_signal_does_not_interrupt_read(void)
{
	int data[2], ready[2];
	CHECK(pipe(data));
	CHECK(pipe(ready));
	pid_t pid = CHECK(fork());
	if (pid == 0) {
		CHECK(close(data[1]));
		CHECK(close(ready[0]));
		sigset_t mask;
		CHECK(sigemptyset(&mask));
		CHECK(sigaddset(&mask, SIGUSR1));
		CHECK(sigprocmask(SIG_BLOCK, &mask, NULL));
		CHECK_WITH(write(ready[1], "r", 1), _ret == 1);
		char byte;
		CHECK_WITH(read(data[0], &byte, 1), _ret == 1 && byte == 'x');
		CHECK(sigpending(&mask));
		CHECK_WITH(sigismember(&mask, SIGUSR1), _ret == 1);
		_exit(0);
	}
	CHECK(close(data[0]));
	CHECK(close(ready[1]));
	char byte;
	CHECK_WITH(read(ready[0], &byte, 1), _ret == 1);
	CHECK(kill(pid, SIGUSR1));
	CHECK_WITH(write(data[1], "x", 1), _ret == 1);
	CHECK(close(data[1]));
	CHECK(close(ready[0]));
	int status;
	CHECK_WITH(waitpid(pid, &status, 0), _ret == pid);
	return !(WIFEXITED(status) && WEXITSTATUS(status) == 0);
}

FN_TEST(syscall_restart)
{
	alarm(60);
	TEST_RES(check_restart(HANDLER_RESTART), _ret == 0);
	TEST_RES(check_restart(HANDLER_INTERRUPT), _ret == 0);
	TEST_RES(check_restart(NOINTR_HANDLER), _ret == 0);
	TEST_RES(check_restart(IGNORED), _ret == 0);
	TEST_RES(check_restart(SUPPRESSED), _ret == 0);
	TEST_RES(check_restart(NO_SIGNAL), _ret == 0);
	TEST_RES(check_restart(NO_SIGNAL_NOINTR), _ret == 0);
	TEST_RES(check_restart(BLOCKED), _ret == 0);
	TEST_RES(check_restart(EDIT_RESULT), _ret == 0);
	TEST_RES(check_restart(EDIT_NUMBER), _ret == 0);
	TEST_RES(check_restart(CANCEL_RESTART), _ret == 0);
	TEST_RES(blocked_signal_does_not_interrupt_read(), _ret == 0);
	alarm(0);
}
END_TEST()
#endif
