// SPDX-License-Identifier: MPL-2.0

#include "seccomp_common.h"
#if defined(__x86_64__)
#include <sys/ptrace.h>
#include <sys/user.h>

enum trace_case {
	REWRITE_DENIED,
	REWRITE_ALLOWED,
	CHANGE_RETURN_ONLY,
	SKIP_ALLOWED,
	SKIP_FILTERED,
	REWRITE_HIGH_BITS,
};

static int trace_syscall(enum trace_case which)
{
	fflush(NULL);
	pid_t child = CHECK(fork());
	if (child == 0) {
		long pid = CHECK(syscall(SYS_getpid));
		no_new_privs();
		deny_syscall(which == SKIP_FILTERED ? -1 : SYS_getppid,
			     SECCOMP_RET_ERRNO | EBUSY);
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
		CHECK(raise(SIGSTOP));
		long result = syscall(which == REWRITE_ALLOWED ? SYS_getppid :
								 SYS_getpid);
		int saved_errno = errno;
		if (which == REWRITE_DENIED || which == SKIP_FILTERED)
			_exit(!(result == -1 && saved_errno == EBUSY));
		if (which == SKIP_ALLOWED)
			_exit(result != 1234);
		_exit(result != pid);
	}

	int status;
	CHECK_WITH(waitpid(child, &status, 0),
		   _ret == child && WIFSTOPPED(status) &&
			   WSTOPSIG(status) == SIGSTOP);
	CHECK(ptrace(PTRACE_SETOPTIONS, child, 0, PTRACE_O_TRACESYSGOOD));
	int entry = 1;
	int changed = 0;
	for (;;) {
		CHECK(ptrace(PTRACE_SYSCALL, child, 0, 0));
		CHECK_WITH(waitpid(child, &status, 0), _ret == child);
		if (WIFEXITED(status) || WIFSIGNALED(status))
			break;
		CHECK_WITH(WSTOPSIG(status), _ret == (SIGTRAP | 0x80));
		struct user_regs_struct regs;
		CHECK(ptrace(PTRACE_GETREGS, child, 0, &regs));
		long target = which == REWRITE_ALLOWED ? SYS_getppid :
							 SYS_getpid;
		if (entry && !changed && regs.orig_rax == target) {
			/* Linux presents -ENOSYS in rax at the entry stop. */
			CHECK_WITH((long)regs.rax, _ret == -ENOSYS);
			switch (which) {
			case REWRITE_DENIED:
				regs.orig_rax = SYS_getppid;
				break;
			case REWRITE_ALLOWED:
				regs.orig_rax = SYS_getpid;
				break;
			case CHANGE_RETURN_ONLY:
				regs.rax = SYS_getppid;
				break;
			case SKIP_ALLOWED:
			case SKIP_FILTERED:
				regs.orig_rax = -1UL;
				regs.rax = 1234;
				break;
			case REWRITE_HIGH_BITS:
				regs.orig_rax = 0x1234567800000000UL |
						SYS_getpid;
				break;
			}
			CHECK(ptrace(PTRACE_SETREGS, child, 0, &regs));
			changed = 1;
		}
		entry = !entry;
	}
	return !(changed && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}

FN_TEST(ptrace_syscall_number)
{
	TEST_RES(trace_syscall(REWRITE_DENIED), _ret == 0);
	TEST_RES(trace_syscall(REWRITE_ALLOWED), _ret == 0);
	TEST_RES(trace_syscall(CHANGE_RETURN_ONLY), _ret == 0);
	TEST_RES(trace_syscall(SKIP_ALLOWED), _ret == 0);
	TEST_RES(trace_syscall(SKIP_FILTERED), _ret == 0);
	TEST_RES(trace_syscall(REWRITE_HIGH_BITS), _ret == 0);
}
END_TEST()
#endif
