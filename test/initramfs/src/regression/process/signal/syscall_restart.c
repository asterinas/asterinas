// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <linux/futex.h>
#include <poll.h>
#include <signal.h>
#include <stdint.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <sys/syscall.h>
#include <sys/time.h>
#include <sys/wait.h>
#include <time.h>
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

FN_TEST(futex_wait_without_timeout_restarts)
{
	signal_count = 0;

	uint32_t *futex_word =
		TEST_RES(mmap(NULL, sizeof(*futex_word), PROT_READ | PROT_WRITE,
			      MAP_SHARED | MAP_ANONYMOUS, -1, 0),
			 _ret != MAP_FAILED);
	*futex_word = 0;

	pid_t parent = getpid();
	pid_t child = TEST_SUCC(fork());
	if (child == 0) {
		sleep(1);
		CHECK(kill(parent, SIGUSR1));
		sleep(1);
		__atomic_store_n(futex_word, 1, __ATOMIC_SEQ_CST);
		CHECK(syscall(SYS_futex, futex_word, FUTEX_WAKE, 1, NULL, NULL,
			      0));
		_exit(0);
	}

	TEST_RES(syscall(SYS_futex, futex_word, FUTEX_WAIT, 0, NULL, NULL, 0),
		 _ret == 0 && signal_count > 0);

	int status = 0;
	TEST_RES(waitpid(child, &status, 0), _ret == child &&
						     WIFEXITED(status) &&
						     WEXITSTATUS(status) == 0);
	TEST_SUCC(munmap(futex_word, sizeof(*futex_word)));
}
END_TEST()

FN_TEST(futex_wait_with_timeout_does_not_restart)
{
	for (int wait_bitset = 0; wait_bitset <= 1; wait_bitset++) {
		signal_count = 0;
		uint32_t futex_word = 0;
		struct timespec timeout;
		if (wait_bitset) {
			TEST_SUCC(clock_gettime(CLOCK_MONOTONIC, &timeout));
			timeout.tv_sec += 3;
		} else {
			timeout.tv_sec = 3;
			timeout.tv_nsec = 0;
		}

		pid_t parent = getpid();
		pid_t child = TEST_SUCC(fork());
		if (child == 0) {
			sleep(1);
			CHECK(kill(parent, SIGUSR1));
			_exit(0);
		}

		int futex_op = wait_bitset ? FUTEX_WAIT_BITSET_PRIVATE :
					     FUTEX_WAIT_PRIVATE;
		uint32_t bitset = wait_bitset ? FUTEX_BITSET_MATCH_ANY : 0;
		TEST_ERRNO(syscall(SYS_futex, &futex_word, futex_op, 0,
				   &timeout, NULL, bitset),
			   EINTR);
		TEST_RES(signal_count > 0, _ret);

		int status = 0;
		TEST_RES(waitpid(child, &status, 0),
			 _ret == child && WIFEXITED(status) &&
				 WEXITSTATUS(status) == 0);
	}
}
END_TEST()

FN_TEST(socket_read_without_timeout_restarts)
{
	signal_count = 0;

	int sockets[2];
	TEST_SUCC(socketpair(AF_UNIX, SOCK_STREAM, 0, sockets));

	pid_t parent = getpid();
	pid_t child = TEST_SUCC(fork());
	if (child == 0) {
		CHECK(close(sockets[0]));
		sleep(1);
		CHECK(kill(parent, SIGUSR1));
		sleep(1);
		CHECK(write(sockets[1], "a", 1));
		_exit(0);
	}

	TEST_SUCC(close(sockets[1]));

	char byte = 0;
	TEST_RES(read(sockets[0], &byte, sizeof(byte)),
		 _ret == 1 && byte == 'a' && signal_count > 0);

	int status = 0;
	TEST_RES(waitpid(child, &status, 0), _ret == child &&
						     WIFEXITED(status) &&
						     WEXITSTATUS(status) == 0);
	TEST_SUCC(close(sockets[0]));
}
END_TEST()

FN_TEST(socket_read_with_timeout_does_not_restart)
{
	signal_count = 0;

	int sockets[2];
	TEST_SUCC(socketpair(AF_UNIX, SOCK_STREAM, 0, sockets));

	struct timeval timeout = { .tv_sec = 5 };
	TEST_SUCC(setsockopt(sockets[0], SOL_SOCKET, SO_RCVTIMEO, &timeout,
			     sizeof(timeout)));

	pid_t parent = getpid();
	pid_t child = TEST_SUCC(fork());
	if (child == 0) {
		CHECK(close(sockets[0]));
		sleep(1);
		CHECK(kill(parent, SIGUSR1));
		sleep(1);
		CHECK(write(sockets[1], "a", 1));
		_exit(0);
	}

	TEST_SUCC(close(sockets[1]));

	char byte = 0;
	TEST_ERRNO(read(sockets[0], &byte, sizeof(byte)), EINTR);
	TEST_RES(signal_count > 0, _ret);

	int status = 0;
	TEST_RES(waitpid(child, &status, 0), _ret == child &&
						     WIFEXITED(status) &&
						     WEXITSTATUS(status) == 0);
	TEST_SUCC(close(sockets[0]));
}
END_TEST()
