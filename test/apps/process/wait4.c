// SPDX-License-Identifier: MPL-2.0

#include "../test.h"

#include <stdio.h>
#include <stdlib.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>
#include <signal.h>
#include <linux/wait.h>
#include <pthread.h>
#include <sys/mman.h>
#include <fcntl.h>

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
		 _ret == 0 && status == 0);
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
		 _ret == 0 && status == 0);
}
END_TEST()

FN_TEST(wait_nowait)
{
	TEST_SUCC(kill(pid, SIGSTOP));

	status = 0;
	TEST_ERRNO(wait4(pid, &status, WSTOPPED | WNOWAIT, NULL), EINVAL);
	status = 0;
	TEST_RES(wait4(pid, &status, WSTOPPED, NULL),
		 _ret == pid && WIFSTOPPED(status) &&
			 WSTOPSIG(status) == SIGSTOP);
	status = 0;
	TEST_RES(wait4(pid, &status, WSTOPPED | WNOHANG, NULL),
		 _ret == 0 && status == 0);

	TEST_SUCC(kill(pid, SIGCONT));

	status = 0;
	TEST_ERRNO(wait4(pid, &status, WCONTINUED | WNOWAIT, NULL), EINVAL);
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
		 _ret == 0 && status == 0);
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
		 _ret == 0 && status == 0);
}
END_TEST()

void *thread_func(void *arg)
{
	while (1) {
		sleep(0.1);
	}
}

void child_process()
{
	pthread_t t1, t2;

	CHECK(pthread_create(&t1, NULL, thread_func, NULL));
	CHECK(pthread_create(&t2, NULL, thread_func, NULL));

	pthread_join(t1, NULL);
	pthread_join(t2, NULL);

	exit(EXIT_SUCCESS);
}

FN_TEST(multithread)
{
	pid_t child_pid = TEST_SUCC(fork());

	if (child_pid == 0) {
		child_process();
	}

	sleep(1);

	TEST_SUCC(kill(child_pid, SIGSTOP));
	int status = 0;
	TEST_RES(wait4(child_pid, &status, WSTOPPED, NULL),
		 _ret == child_pid && WIFSTOPPED(status) &&
			 WSTOPSIG(status) == SIGSTOP);
	TEST_RES(wait4(child_pid, &status, WSTOPPED | WNOHANG, NULL),
		 _ret == 0);

	TEST_SUCC(kill(child_pid, SIGKILL));
	TEST_RES(wait4(child_pid, NULL, 0, NULL), _ret == child_pid);
}
END_TEST()

volatile int sigint_counter = 0;
volatile int sigtrap_counter = 0;

void handle_sigint(int signum, siginfo_t *_info, void *_context)
{
	sigint_counter += 1;
}

void handle_sigtrap(int signum, siginfo_t *_info, void *_context)
{
	sigtrap_counter += 1;
}

void child_process2(int *pipe_fds)
{
	CHECK(close(pipe_fds[0]));

	struct sigaction new_action = {};
	new_action.sa_sigaction = handle_sigint;
	CHECK(sigaction(SIGINT, &new_action, NULL));

	new_action.sa_sigaction = handle_sigtrap;
	CHECK(sigaction(SIGTRAP, &new_action, NULL));

	while (1) {
		usleep(100);
		if (sigint_counter == 1) {
			sigint_counter = 0;
			CHECK(write(pipe_fds[1], "a", 1));
		}
		if (sigtrap_counter == 1) {
			sigtrap_counter = 0;
			CHECK(write(pipe_fds[1], "b", 1));
		}
	}

	exit(EXIT_SUCCESS);
}

FN_TEST(nested_signals)
{
	int pipe_fds[2];
	TEST_SUCC(pipe(pipe_fds));

	for (int i = 0; i < 2; i++) {
		int fd = pipe_fds[i];
		int flags = TEST_SUCC(fcntl(fd, F_GETFL, 0));
		TEST_SUCC(fcntl(fd, F_SETFL, flags | O_NONBLOCK));
	}

	int child_pid = TEST_SUCC(fork());

	if (child_pid == 0) {
		child_process2(pipe_fds);
	}

	TEST_SUCC(close(pipe_fds[1]));
	sleep(1);

	char buf[1] = { 0 };

	// SIGINT -> SIGTRAP
	TEST_SUCC(kill(child_pid, SIGINT));
	sleep(1);
	TEST_RES(read(pipe_fds[0], buf, 1), _ret == 1 && buf[0] == 'a');

	TEST_SUCC(kill(child_pid, SIGTRAP));
	sleep(1);
	TEST_RES(read(pipe_fds[0], buf, 1), _ret == 1 && buf[0] == 'b');

	// SIGSTOP -> SIGINT -> SIGTRAP -> SIGCONT
	TEST_SUCC(kill(child_pid, SIGSTOP));
	sleep(1);

	// FIXME: The following two read pipe checks are commented out because
	// Asterinas currently handles signals with user-provided handlers
	// when the thread is stopped, while Linux does not.
	TEST_SUCC(kill(child_pid, SIGINT));
	sleep(1);
	// TEST_ERRNO(read(pipe_fds[0], buf, 1), EAGAIN);

	TEST_SUCC(kill(child_pid, SIGTRAP));
	sleep(1);
	// TEST_ERRNO(read(pipe_fds[0], buf, 1), EAGAIN);

	TEST_RES(wait4(child_pid, &status, WSTOPPED, NULL),
		 _ret == child_pid && WIFSTOPPED(status) &&
			 WSTOPSIG(status) == SIGSTOP);

	TEST_SUCC(kill(child_pid, SIGCONT));
	sleep(1);
	TEST_RES(read(pipe_fds[0], buf, 1), _ret == 1 && buf[0] == 'a');
	TEST_RES(read(pipe_fds[0], buf, 1), _ret == 1 && buf[0] == 'b');

	TEST_RES(wait4(child_pid, &status, WCONTINUED, NULL),
		 _ret == child_pid && WIFCONTINUED(status));

	// SIGKILL
	TEST_SUCC(kill(child_pid, SIGKILL));
	TEST_RES(wait4(child_pid, NULL, 0, NULL), _ret == child_pid);
}
END_TEST()

FN_SETUP(kill_stopped)
{
	CHECK(kill(pid, SIGKILL));
	sleep(1);
	CHECK_WITH(wait4(pid, &status, WSTOPPED, NULL),
		   _ret == pid && WIFSIGNALED(status) &&
			   WTERMSIG(status) == SIGKILL);
}
END_SETUP()
