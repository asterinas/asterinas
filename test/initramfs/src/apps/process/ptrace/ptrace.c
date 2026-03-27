// SPDX-License-Identifier: MPL-2.0

#include <signal.h>
#include <stdlib.h>
#include <sys/ptrace.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"
#include "../../common/yama_ptrace_scope.h"

FN_TEST(ptrace_signal_stop_wait_continue)
{
	SKIP_TEST_IF(read_yama_scope() == YAMA_SCOPE_NO_ATTACH);

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
		CHECK(raise(SIGTERM));
		exit(0);
	}

	int status = 0;
	TEST_RES(waitpid(pid, &status, 0), _ret == pid && WIFSTOPPED(status) &&
						   WSTOPSIG(status) == SIGTERM);
	TEST_SUCC(ptrace(PTRACE_CONT, pid, 0, 0));
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()

FN_TEST(ptrace_kill_from_signal_stop)
{
	SKIP_TEST_IF(read_yama_scope() == YAMA_SCOPE_NO_ATTACH);

	int *flag = TEST_SUCC(mmap(NULL, 4096, PROT_READ | PROT_WRITE,
				   MAP_SHARED | MAP_ANONYMOUS, -1, 0));

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		// Trigger a page fault first.
		*flag = 1;
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
		CHECK(raise(SIGTERM));
		// This write is unreachable, because the child is in a ptrace-stop,
		// and the parent has sent a `SIGKILL` to it.
		*flag = 2;
		exit(-1);
	}

	int status;
	siginfo_t si;
	TEST_RES(waitpid(pid, &status, 0), _ret == pid && WIFSTOPPED(status) &&
						   WSTOPSIG(status) == SIGTERM);
	TEST_RES(ptrace(PTRACE_GETSIGINFO, pid, 0, &si),
		 _ret == 0 && si.si_signo == SIGTERM);

	TEST_SUCC(ptrace(PTRACE_KILL, pid, 0, 0));
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFSIGNALED(status) &&
			 WTERMSIG(status) == SIGKILL && *flag == 1);
	TEST_SUCC(munmap((void *)flag, 4096));
}
END_TEST()
