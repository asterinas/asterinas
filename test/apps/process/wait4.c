// SPDX-License-Identifier: MPL-2.0

#include "../network/test.h"

#include <stdio.h>
#include <stdlib.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>
#include <signal.h>
#include <linux/wait.h>

static pid_t pid;
static int status;

FN_SETUP(fork_child)
{
	pid = CHECK(fork());

	if (pid == 0) {
		// Child entering an infinite loop until killed by parent.
		while (1) {
			usleep(100);
		}

		exit(EXIT_SUCCESS);
	}

	// Parent process
	sleep(1); // Ensure the child process is running
}
END_SETUP()

FN_TEST(stop_child)
{
	// Stop the child process
	TEST_SUCC(kill(pid, SIGSTOP));
	TEST_RES(wait4(pid, &status, WSTOPPED, NULL),
		 _ret == pid && WIFSTOPPED(status) &&
			 WSTOPSIG(status) == SIGSTOP);
	status = 0;
	TEST_RES(wait4(pid, &status, WSTOPPED | WNOHANG, NULL),
		 _ret == 0 && !WIFSTOPPED(status));
}
END_TEST()

FN_TEST(continue_child)
{
	TEST_SUCC(kill(pid, SIGCONT));
	status = 0;
	TEST_RES(wait4(pid, &status, WCONTINUED, NULL),
		 _ret == pid && WIFCONTINUED(status));
	status = 0;
	TEST_RES(wait4(pid, &status, WCONTINUED | WNOHANG, NULL),
		 _ret == 0 && !WIFCONTINUED(status));
}
END_TEST()

FN_TEST(wait_nowait)
{
	TEST_SUCC(kill(pid, SIGSTOP));

	status = 0;
	TEST_RES(wait4(pid, &status, WSTOPPED | WNOWAIT, NULL),
		 _ret == pid && WIFSTOPPED(status) &&
			 WSTOPSIG(status) == SIGSTOP);
	status = 0;
	TEST_RES(wait4(pid, &status, WSTOPPED, NULL),
		 _ret == pid && WIFSTOPPED(status) &&
			 WSTOPSIG(status) == SIGSTOP);
	status = 0;
	TEST_RES(wait4(pid, &status, WSTOPPED | WNOHANG, NULL),
		 _ret == 0 && status == 0);

	TEST_SUCC(kill(pid, SIGCONT));

	status = 0;
	TEST_RES(wait4(pid, &status, WCONTINUED | WNOWAIT, NULL),
		 _ret == pid && WIFCONTINUED(status));
	status = 0;
	TEST_RES(wait4(pid, &status, WCONTINUED, NULL),
		 _ret == pid && WIFCONTINUED(status));
	status = 0;
	TEST_RES(wait4(pid, &status, WCONTINUED | WNOHANG, NULL),
		 _ret == 0 && status == 0);
}
END_TEST()

FN_TEST(wait_stopped_and_continued)
{
	TEST_SUCC(kill(pid, SIGSTOP));
	sleep(1);
	TEST_SUCC(kill(pid, SIGCONT));
	sleep(1);

	status = 0;
	TEST_RES(wait4(pid, &status, WSTOPPED | WCONTINUED, NULL),
		 _ret == pid && WIFCONTINUED(status));
	status = 0;
	TEST_RES(wait4(pid, &status, WSTOPPED | WCONTINUED | WNOHANG, NULL),
		 _ret == 0 && status == 0);
}
END_TEST()

FN_TEST(continue_not_stopped)
{
	TEST_SUCC(kill(pid, SIGCONT));
	status = 0;
	TEST_RES(wait4(pid, &status, WCONTINUED | WNOHANG, NULL),
		 _ret == 0 && !WIFCONTINUED(status));
}
END_TEST()

FN_TEST(stop_continue_continue)
{
	TEST_SUCC(kill(pid, SIGSTOP));
	sleep(1);
	TEST_SUCC(kill(pid, SIGCONT));
	sleep(1);
	TEST_SUCC(kill(pid, SIGCONT));
	sleep(1);

	status = 0;
	TEST_RES(wait4(pid, &status, WSTOPPED | WNOHANG, NULL),
		 status == 0 && _ret == 0);
	status = 0;
	TEST_RES(wait4(pid, &status, WSTOPPED | WCONTINUED, NULL),
		 _ret == pid && WIFCONTINUED(status));
	status = 0;
	TEST_RES(wait4(pid, &status, WSTOPPED | WCONTINUED | WNOHANG, NULL),
		 _ret == 0 && status == 0);
}
END_TEST()

FN_TEST(stop_continue_stop)
{
	TEST_SUCC(kill(pid, SIGSTOP));
	sleep(1);
	TEST_SUCC(kill(pid, SIGCONT));
	sleep(1);
	TEST_SUCC(kill(pid, SIGSTOP));
	sleep(1);

	status = 0;
	TEST_RES(wait4(pid, &status, WCONTINUED | WNOHANG, NULL), status == 0);
	status = 0;
	TEST_RES(wait4(pid, &status, WSTOPPED, NULL),
		 _ret == pid && WIFSTOPPED(status) &&
			 WSTOPSIG(status) == SIGSTOP);
	status = 0;
	TEST_RES(wait4(pid, &status, WSTOPPED | WNOHANG, NULL),
		 _ret == 0 && status == 0);

	// Restore the state
	TEST_SUCC(kill(pid, SIGCONT));
	status = 0;
	TEST_RES(wait4(pid, &status, WCONTINUED, NULL),
		 _ret == pid && WIFCONTINUED(status));
	status = 0;
	TEST_RES(wait4(pid, &status, WCONTINUED | WSTOPPED | WNOHANG, NULL),
		 _ret == 0 && status == 0);
}
END_TEST()

FN_TEST(stop_stopped)
{
	TEST_SUCC(kill(pid, SIGSTOP));
	status = 0;
	TEST_RES(wait4(pid, &status, WSTOPPED, NULL),
		 _ret == pid && WIFSTOPPED(status) &&
			 WSTOPSIG(status) == SIGSTOP);

	TEST_SUCC(kill(pid, SIGSTOP));
	sleep(1);
	status = 0;
	TEST_RES(wait4(pid, &status, WSTOPPED | WNOHANG, NULL),
		 _ret == 0 && !WIFSTOPPED(status));
}
END_TEST()

FN_SETUP(kill_stopped)
{
	kill(pid, SIGKILL);
	sleep(1);
	CHECK_WITH(wait4(pid, &status, WSTOPPED, NULL),
		   _ret == pid && WIFSIGNALED(status) &&
			   WTERMSIG(status) == SIGKILL);
}
END_SETUP()
