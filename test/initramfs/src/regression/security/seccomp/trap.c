// SPDX-License-Identifier: MPL-2.0

#include "seccomp_common.h"

#include <ucontext.h>
#include <sys/ptrace.h>

static volatile sig_atomic_t caught;
static volatile sig_atomic_t expected_nr;
static volatile sig_atomic_t expected_data;

static void trap_handler(int signum, siginfo_t *info, void *context)
{
	if (signum != SIGSYS || info->si_signo != SIGSYS ||
	    info->si_code != 1 || info->si_errno != expected_data ||
	    info->si_syscall != expected_nr || info->si_arch != NATIVE_ARCH ||
	    info->si_call_addr == NULL)
		_exit(90);
#if defined(__x86_64__)
	ucontext_t *uc = context;
	uintptr_t expected_ip = (uintptr_t)info->si_call_addr;
	/* Linux x86 interprets a rolled-back -ERESTARTSYS with SA_RESTART. */
	if (expected_nr == -512)
		expected_ip -= 2;
	if ((int)uc->uc_mcontext.gregs[REG_RAX] != expected_nr ||
	    (uintptr_t)uc->uc_mcontext.gregs[REG_RIP] != expected_ip)
		_exit(91);
	uc->uc_mcontext.gregs[REG_RIP] = (greg_t)info->si_call_addr;
	uc->uc_mcontext.gregs[REG_RAX] = 1234;
#endif
	++caught;
}

static void set_handler(void)
{
	struct sigaction action = { .sa_sigaction = trap_handler,
				    .sa_flags = SA_SIGINFO | SA_RESTART };
	sigemptyset(&action.sa_mask);
	CHECK(sigaction(SIGSYS, &action, NULL));
}

static void handled_trap(void)
{
	set_handler();
	no_new_privs();
	expected_nr = SYS_getppid;
	expected_data = 0xfedc;
	deny_syscall(expected_nr, SECCOMP_RET_TRAP | expected_data);
	/* TRAP wins over ERRNO regardless of installation order. */
	deny_syscall(expected_nr, SECCOMP_RET_ERRNO | EPERM);
	long result = syscall(expected_nr);
#if defined(__x86_64__)
	CHECK_WITH(result, _ret == 1234);
#else
	(void)result;
#endif
	CHECK_WITH(caught, _ret == 1);
	expected_data = 17;
	deny_syscall(expected_nr, SECCOMP_RET_TRAP | expected_data);
	syscall(expected_nr);
	CHECK_WITH(caught, _ret == 2);
}

static void negative_number(void)
{
	set_handler();
	no_new_privs();
	/* Exercise the overlap between syscall rollback and SA_RESTART. */
	expected_nr = -512;
	expected_data = 9;
	deny_syscall(expected_nr, SECCOMP_RET_TRAP | expected_data);
	syscall(expected_nr);
	CHECK_WITH(caught, _ret == 1);
}

static void default_trap(void)
{
	no_new_privs();
	deny_syscall(SYS_getppid, SECCOMP_RET_TRAP);
	syscall(SYS_getppid);
}

static void ignored_trap(void)
{
	CHECK_WITH(signal(SIGSYS, SIG_IGN), _ret != SIG_ERR);
	default_trap();
}

static void blocked_trap(void)
{
	set_handler();
	sigset_t mask;
	sigemptyset(&mask);
	sigaddset(&mask, SIGSYS);
	CHECK(sigprocmask(SIG_BLOCK, &mask, NULL));
	default_trap();
}

static int trap_skips_write(void)
{
	int pipefd[2];
	CHECK(pipe(pipefd));
	pid_t child = CHECK(fork());
	if (!child) {
		CHECK(close(pipefd[0]));
		set_handler();
		no_new_privs();
		expected_nr = SYS_write;
		expected_data = 0;
		deny_syscall(SYS_write, SECCOMP_RET_TRAP);
		syscall(SYS_write, pipefd[1], "x", 1);
		_exit(caught != 1);
	}
	CHECK(close(pipefd[1]));
	char byte;
	int result = CHECK(read(pipefd[0], &byte, 1));
	int status;
	CHECK_WITH(waitpid(child, &status, 0), _ret == child);
	CHECK(close(pipefd[0]));
	return result != 0 || !WIFEXITED(status) || WEXITSTATUS(status) != 0;
}

static int tracer_delivers_or_suppresses(int deliver)
{
	fflush(NULL);
	pid_t child = CHECK(fork());
	if (!child) {
		set_handler();
		no_new_privs();
		expected_nr = SYS_getppid;
		expected_data = 123;
		deny_syscall(expected_nr, SECCOMP_RET_TRAP | expected_data);
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
		CHECK(raise(SIGSTOP));
		long result = syscall(SYS_getppid);
#if defined(__x86_64__)
		if (result != (deliver ? 1234 : SYS_getppid))
			_exit(92);
#else
		(void)result;
#endif
		_exit(caught != deliver);
	}
	int status;
	CHECK_WITH(waitpid(child, &status, 0),
		   _ret == child && WIFSTOPPED(status) &&
			   WSTOPSIG(status) == SIGSTOP);
	CHECK(ptrace(PTRACE_CONT, child, 0, 0));
	CHECK_WITH(waitpid(child, &status, 0),
		   _ret == child && WIFSTOPPED(status) &&
			   WSTOPSIG(status) == SIGSYS);
	siginfo_t info;
	CHECK(ptrace(PTRACE_GETSIGINFO, child, 0, &info));
	int failed = info.si_signo != SIGSYS || info.si_code != 1 ||
		     info.si_errno != 123 || info.si_syscall != SYS_getppid ||
		     info.si_arch != NATIVE_ARCH || info.si_call_addr == NULL;
	CHECK(ptrace(PTRACE_CONT, child, 0, deliver ? SIGSYS : 0));
	CHECK_WITH(waitpid(child, &status, 0), _ret == child);
	return failed || !WIFEXITED(status) || WEXITSTATUS(status) != 0;
}

FN_TEST(trap)
{
	uint32_t action = SECCOMP_RET_TRAP;
	TEST_SUCC(syscall(SYS_seccomp, SECCOMP_GET_ACTION_AVAIL, 0, &action));
	TEST_RES(run_child(handled_trap, 0), _ret == 0);
	TEST_RES(run_child(negative_number, 0), _ret == 0);
	TEST_RES(run_child(default_trap, SIGSYS), _ret == 0);
	TEST_RES(run_child(ignored_trap, SIGSYS), _ret == 0);
	TEST_RES(run_child(blocked_trap, SIGSYS), _ret == 0);
	TEST_RES(trap_skips_write(), _ret == 0);
	TEST_RES(tracer_delivers_or_suppresses(1), _ret == 0);
	TEST_RES(tracer_delivers_or_suppresses(0), _ret == 0);
}
END_TEST()
