// SPDX-License-Identifier: MPL-2.0

#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/ptrace.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"
#include "../../common/yama_ptrace_scope.h"

static void exit_with_233_handler(int sig)
{
	(void)sig;
	_exit(233);
}

static void install_handler(int sig, void (*handler)(int))
{
	struct sigaction action = {
		.sa_handler = handler,
		.sa_flags = 0,
	};

	CHECK(sigemptyset(&action.sa_mask));
	CHECK(sigaction(sig, &action, NULL));
}

static int read_tracer_pid(pid_t pid)
{
	char path[64];
	char line[256];
	FILE *status_file;
	int tracer_pid = -1;

	snprintf(path, sizeof(path), "/proc/%d/status", pid);
	status_file = fopen(path, "r");
	CHECK_WITH(0, status_file != NULL);

	while (fgets(line, sizeof(line), status_file) != NULL) {
		if (strncmp(line, "TracerPid:\t", 11) == 0) {
			tracer_pid = atoi(line + 11);
			break;
		}
	}

	CHECK(fclose(status_file));
	return tracer_pid;
}

FN_TEST(ptrace_signal_stop_wait_continue)
{
	SKIP_TEST_IF(read_yama_scope() == YAMA_SCOPE_NO_ATTACH);

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
		CHECK(raise(SIGTERM));
		_exit(0);
	}

	int status = 0;
	TEST_RES(waitpid(pid, &status, 0), _ret == pid && WIFSTOPPED(status) &&
						   WSTOPSIG(status) == SIGTERM);
	TEST_SUCC(ptrace(PTRACE_CONT, pid, 0, 0));
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()

FN_TEST(ptrace_sigkill_interrupts_ptrace_stop)
{
	SKIP_TEST_IF(read_yama_scope() == YAMA_SCOPE_NO_ATTACH);

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
		CHECK(raise(SIGSTOP));
		_exit(1);
	}

	int status = 0;
	TEST_RES(waitpid(pid, &status, 0), _ret == pid && WIFSTOPPED(status) &&
						   WSTOPSIG(status) == SIGSTOP);
	TEST_SUCC(kill(pid, SIGKILL));
	TEST_RES(waitpid(pid, &status, 0), _ret == pid && WIFSIGNALED(status) &&
						   WTERMSIG(status) == SIGKILL);
}
END_TEST()

FN_TEST(ptrace_execve_reports_sigtrap)
{
	SKIP_TEST_IF(read_yama_scope() == YAMA_SCOPE_NO_ATTACH);

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
		CHECK(execl("/bin/true", "/bin/true", NULL));
		_exit(1);
	}

	int status = 0;
	TEST_RES(waitpid(pid, &status, 0), _ret == pid && WIFSTOPPED(status) &&
						   WSTOPSIG(status) == SIGTRAP);
	TEST_SUCC(ptrace(PTRACE_CONT, pid, 0, 0));
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()

FN_TEST(ptrace_tracer_exit_does_not_reinject_waited_signal)
{
	SKIP_TEST_IF(read_yama_scope() == YAMA_SCOPE_NO_ATTACH);

	int pid_pipe[2];
	TEST_SUCC(prctl(PR_SET_CHILD_SUBREAPER, 1));
	TEST_SUCC(pipe(pid_pipe));

	pid_t tracer_pid = TEST_SUCC(fork());
	if (tracer_pid == 0) {
		pid_t child_pid;
		int status = 0;

		CHECK(close(pid_pipe[0]));
		child_pid = CHECK(fork());
		if (child_pid == 0) {
			CHECK(close(pid_pipe[1]));
			CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
			CHECK(raise(SIGTERM));
			_exit(1);
		}

		CHECK(write(pid_pipe[1], &child_pid, sizeof(child_pid)));
		CHECK(close(pid_pipe[1]));
		CHECK_WITH(waitpid(child_pid, &status, 0),
			   _ret == child_pid && WIFSTOPPED(status) &&
				   WSTOPSIG(status) == SIGTERM);
		_exit(0);
	}

	pid_t tracee_pid;
	TEST_SUCC(close(pid_pipe[1]));
	TEST_RES(read(pid_pipe[0], &tracee_pid, sizeof(tracee_pid)),
		 _ret == sizeof(tracee_pid));
	TEST_SUCC(close(pid_pipe[0]));

	int status = 0;
	TEST_RES(waitpid(tracer_pid, &status, 0),
		 _ret == tracer_pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == 0);
	TEST_RES(waitpid(tracee_pid, &status, 0),
		 _ret == tracee_pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == 1);
}
END_TEST()

FN_TEST(ptrace_tracer_exit_reinjects_unwaited_signal)
{
	SKIP_TEST_IF(read_yama_scope() == YAMA_SCOPE_NO_ATTACH);

	int pid_pipe[2];
	TEST_SUCC(prctl(PR_SET_CHILD_SUBREAPER, 1));
	TEST_SUCC(pipe(pid_pipe));

	pid_t tracer_pid = TEST_SUCC(fork());
	if (tracer_pid == 0) {
		CHECK(close(pid_pipe[0]));
		pid_t tracee_pid = CHECK(fork());
		if (tracee_pid == 0) {
			CHECK(close(pid_pipe[1]));
			install_handler(SIGUSR1, exit_with_233_handler);
			CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
			CHECK(raise(SIGUSR1));
			_exit(-1);
		}

		CHECK(write(pid_pipe[1], &tracee_pid, sizeof(tracee_pid)));
		CHECK(close(pid_pipe[1]));
		// Wait for the tracee to stop on the signal without consuming it.
		siginfo_t info = { 0 };
		CHECK_WITH(waitid(P_PID, tracee_pid, &info, WSTOPPED | WNOWAIT),
			   _ret == 0 && info.si_code == CLD_TRAPPED &&
				   info.si_status == SIGUSR1);
		_exit(0);
	}

	pid_t tracee_pid;
	TEST_SUCC(close(pid_pipe[1]));
	TEST_RES(read(pid_pipe[0], &tracee_pid, sizeof(tracee_pid)),
		 _ret == sizeof(tracee_pid));
	TEST_SUCC(close(pid_pipe[0]));

	int status = 0;
	TEST_RES(waitpid(tracer_pid, &status, 0),
		 _ret == tracer_pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == 0);
	TEST_RES(waitpid(tracee_pid, &status, 0),
		 _ret == tracee_pid && WIFEXITED(status) &&
			 WEXITSTATUS(status) == 233);
}
END_TEST()

FN_TEST(ptrace_cont_injects_signal)
{
	SKIP_TEST_IF(read_yama_scope() == YAMA_SCOPE_NO_ATTACH);

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		install_handler(SIGUSR2, exit_with_233_handler);
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
		CHECK(raise(SIGUSR1));
		_exit(1);
	}

	int status = 0;
	TEST_RES(waitpid(pid, &status, 0), _ret == pid && WIFSTOPPED(status) &&
						   WSTOPSIG(status) == SIGUSR1);
	TEST_SUCC(ptrace(PTRACE_CONT, pid, 0, SIGUSR2));
	TEST_RES(waitpid(pid, &status, 0), _ret == pid && WIFEXITED(status) &&
						   WEXITSTATUS(status) == 233);
}
END_TEST()

FN_TEST(ptrace_tracee_exit_without_stop)
{
	SKIP_TEST_IF(read_yama_scope() == YAMA_SCOPE_NO_ATTACH);

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
		_exit(7);
	}

	int status = 0;
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 7);
}
END_TEST()

FN_TEST(ptrace_proc_pid_status_reports_tracer_pid)
{
	SKIP_TEST_IF(read_yama_scope() == YAMA_SCOPE_NO_ATTACH);

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		if (read_tracer_pid(getpid()) != 0) {
			_exit(1);
		}
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
		if (read_tracer_pid(getpid()) != getppid()) {
			_exit(2);
		}
		CHECK(raise(SIGSTOP));
		_exit(0);
	}

	int status = 0;
	TEST_RES(waitpid(pid, &status, 0), _ret == pid && WIFSTOPPED(status) &&
						   WSTOPSIG(status) == SIGSTOP);
	TEST_RES(read_tracer_pid(pid), _ret == getpid());
	TEST_SUCC(ptrace(PTRACE_CONT, pid, 0, 0));
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()

FN_TEST(ptrace_invalid_op_fails_with_eio)
{
	SKIP_TEST_IF(read_yama_scope() == YAMA_SCOPE_NO_ATTACH);

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
		CHECK(raise(SIGSTOP));
		_exit(0);
	}

	int status = 0;
	TEST_RES(waitpid(pid, &status, 0), _ret == pid && WIFSTOPPED(status) &&
						   WSTOPSIG(status) == SIGSTOP);
	TEST_ERRNO(ptrace(0x3c3c3c3c, pid, 0, 0), EIO);
	TEST_SUCC(ptrace(PTRACE_CONT, pid, 0, 0));
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()

FN_TEST(ptrace_double_traceme_fails_with_eperm)
{
	SKIP_TEST_IF(read_yama_scope() == YAMA_SCOPE_NO_ATTACH);

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
		errno = 0;
		if (ptrace(PTRACE_TRACEME, 0, 0, 0) != -1 || errno != EPERM) {
			_exit(1);
		}
		_exit(0);
	}

	int status = 0;
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
}
END_TEST()

FN_TEST(ptrace_cont_non_stopped_tracee_fails_with_esrch)
{
	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		sleep(30);
		_exit(0);
	}

	int status = 0;
	TEST_ERRNO(ptrace(PTRACE_CONT, pid, 0, 0), ESRCH);
	TEST_SUCC(kill(pid, SIGKILL));
	TEST_RES(waitpid(pid, &status, 0), _ret == pid && WIFSIGNALED(status));
}
END_TEST()

FN_TEST(ptrace_wait_nohang_on_running_tracee)
{
	SKIP_TEST_IF(read_yama_scope() == YAMA_SCOPE_NO_ATTACH);

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
		CHECK(raise(SIGSTOP));
		for (;;) {
			pause();
		}
	}

	int status = 0;
	TEST_RES(waitpid(pid, &status, 0), _ret == pid && WIFSTOPPED(status) &&
						   WSTOPSIG(status) == SIGSTOP);
	TEST_SUCC(ptrace(PTRACE_CONT, pid, 0, 0));
	TEST_RES(waitpid(pid, &status, WNOHANG), _ret == 0);
	TEST_SUCC(kill(pid, SIGKILL));
	TEST_RES(waitpid(pid, &status, 0), _ret == pid && WIFSIGNALED(status));
}
END_TEST()

FN_TEST(ptrace_wait_nowait_does_not_consume_ptrace_stop)
{
	SKIP_TEST_IF(read_yama_scope() == YAMA_SCOPE_NO_ATTACH);

	pid_t pid = TEST_SUCC(fork());
	if (pid == 0) {
		CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
		CHECK(raise(SIGUSR1));
		_exit(1);
	}

	siginfo_t info = { 0 };

	TEST_RES(waitid(P_PID, pid, &info, WSTOPPED | WNOWAIT),
		 _ret == 0 && info.si_code == CLD_TRAPPED &&
			 info.si_status == SIGUSR1);
	int status = 0;
	TEST_RES(waitpid(pid, &status, 0), _ret == pid && WIFSTOPPED(status) &&
						   WSTOPSIG(status) == SIGUSR1);
	TEST_SUCC(ptrace(PTRACE_CONT, pid, 0, 0));
	TEST_RES(waitpid(pid, &status, 0),
		 _ret == pid && WIFEXITED(status) && WEXITSTATUS(status) == 1);
}
END_TEST()

FN_TEST(ptrace_multiple_tracees)
{
	SKIP_TEST_IF(read_yama_scope() == YAMA_SCOPE_NO_ATTACH);

	const int TRACEE_COUNT = 8;
	pid_t pids[8];
	for (int i = 0; i < TRACEE_COUNT; i++) {
		pids[i] = TEST_SUCC(fork());
		if (pids[i] == 0) {
			CHECK(ptrace(PTRACE_TRACEME, 0, 0, 0));
			CHECK(raise(SIGTERM));
			_exit(0);
		}
	}

	int stop_count = 0;
	while (stop_count < TRACEE_COUNT) {
		int status;
		pid_t waited = TEST_RES(waitpid(0, &status, 0),
					WIFSTOPPED(status) &&
						WSTOPSIG(status) == SIGTERM);
		int found = 0;
		for (int i = 0; i < TRACEE_COUNT; i++) {
			if (waited == pids[i]) {
				found = 1;
				break;
			}
		}
		TEST_RES(found, _ret == 1);
		stop_count++;
	}

	for (int i = 0; i < TRACEE_COUNT; i++) {
		TEST_SUCC(ptrace(PTRACE_CONT, pids[i], 0, 0));
	}

	int exit_count = 0;
	while (exit_count < TRACEE_COUNT) {
		int status;
		pid_t waited =
			TEST_RES(waitpid(0, &status, 0),
				 WIFEXITED(status) && WEXITSTATUS(status) == 0);
		int found = 0;
		for (int i = 0; i < TRACEE_COUNT; i++) {
			if (waited == pids[i]) {
				found = 1;
				break;
			}
		}
		TEST_RES(found, _ret == 1);
		exit_count++;
	}
}
END_TEST()
