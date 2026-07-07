// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <poll.h>
#include <signal.h>
#include <stdlib.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../common/test.h"

static volatile sig_atomic_t signal_count;

static void signal_handler(int signum)
{
	signal_count++;
}

FN_SETUP(install_restart_handler)
{
	struct sigaction action = {};
	action.sa_handler = signal_handler;
	action.sa_flags = SA_RESTART;
	CHECK(sigaction(SIGUSR1, &action, NULL));
}
END_SETUP()

FN_TEST(read_restarts_with_sa_restart)
{
	signal_count = 0;

	int pipefds[2];
	TEST_SUCC(pipe(pipefds));

	pid_t parent = getpid();
	pid_t child = TEST_SUCC(fork());
	if (child == 0) {
		CHECK(close(pipefds[0]));
		sleep(1);
		CHECK(kill(parent, SIGUSR1));
		sleep(1);
		CHECK(write(pipefds[1], "a", 1));
		_exit(0);
	}

	TEST_SUCC(close(pipefds[1]));

	char byte = 0;
	TEST_RES(read(pipefds[0], &byte, sizeof(byte)),
		 _ret == 1 && byte == 'a' && signal_count > 0);

	int status = 0;
	TEST_RES(waitpid(child, &status, 0), _ret == child &&
						     WIFEXITED(status) &&
						     WEXITSTATUS(status) == 0);
	TEST_SUCC(close(pipefds[0]));
}
END_TEST()

FN_TEST(poll_does_not_restart_with_sa_restart)
{
	signal_count = 0;

	int pipefds[2];
	TEST_SUCC(pipe(pipefds));

	pid_t parent = getpid();
	pid_t child = TEST_SUCC(fork());
	if (child == 0) {
		CHECK(close(pipefds[0]));
		sleep(1);
		CHECK(kill(parent, SIGUSR1));
		sleep(1);
		CHECK(write(pipefds[1], "a", 1));
		_exit(0);
	}

	TEST_SUCC(close(pipefds[1]));

	struct pollfd pfd = {
		.fd = pipefds[0],
		.events = POLLIN,
	};
	TEST_ERRNO(poll(&pfd, 1, -1), EINTR);

	int status = 0;
	TEST_RES(waitpid(child, &status, 0), _ret == child &&
						     WIFEXITED(status) &&
						     WEXITSTATUS(status) == 0);
	TEST_SUCC(close(pipefds[0]));
}
END_TEST()
