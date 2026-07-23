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
#include <sys/uio.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#include "../../common/test.h"

#define TEST_TIMEOUT_SECONDS 10
#define SIGNAL_INTERVAL_NS 100000000

static volatile sig_atomic_t signal_count;
static timer_t signal_timer;

// Repeat delivery so a signal arriving before syscall entry cannot be lost.
static const struct itimerspec periodic_timer = {
	.it_interval.tv_nsec = SIGNAL_INTERVAL_NS,
	.it_value.tv_nsec = SIGNAL_INTERVAL_NS,
};
static const struct itimerspec stopped_timer = {};

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

	struct sigevent event = {
		.sigev_notify = SIGEV_SIGNAL,
		.sigev_signo = SIGUSR1,
	};
	CHECK(timer_create(CLOCK_MONOTONIC, &event, &signal_timer));

	// SIGALRM terminates a stuck test independently of the SIGUSR1 timer.
	action.sa_handler = SIG_DFL;
	action.sa_flags = 0;
	CHECK(sigaction(SIGALRM, &action, NULL));
}
END_SETUP()

FN_TEST(read_and_readv_restart_with_sa_restart)
{
	for (int use_readv = 0; use_readv <= 1; use_readv++) {
		signal_count = 0;
		TEST_SUCC(alarm(TEST_TIMEOUT_SECONDS));

		int pipefds[2];
		TEST_SUCC(pipe(pipefds));

		pid_t parent = getpid();
		pid_t child = TEST_SUCC(fork());
		if (child == 0) {
			CHECK(close(pipefds[0]));
			sleep(1);
			CHECK(kill(parent, SIGUSR1));
			sleep(1);
			CHECK(write(pipefds[1], "ab", 2));
			_exit(0);
		}

		TEST_SUCC(close(pipefds[1]));

		char bytes[2] = {};
		if (use_readv) {
			struct iovec iov[] = {
				{ .iov_base = &bytes[0], .iov_len = 1 },
				{ .iov_base = &bytes[1], .iov_len = 1 },
			};
			TEST_RES(readv(pipefds[0], iov, 2),
				 _ret == 2 && bytes[0] == 'a' &&
					 bytes[1] == 'b' && signal_count > 0);
		} else {
			TEST_RES(read(pipefds[0], bytes, sizeof(bytes)),
				 _ret == 2 && bytes[0] == 'a' &&
					 bytes[1] == 'b' && signal_count > 0);
		}

		int status = 0;
		TEST_RES(waitpid(child, &status, 0),
			 _ret == child && WIFEXITED(status) &&
				 WEXITSTATUS(status) == 0);
		TEST_SUCC(close(pipefds[0]));
		TEST_SUCC(alarm(0));
	}
}
END_TEST()

FN_TEST(poll_does_not_restart_with_sa_restart)
{
	signal_count = 0;
	TEST_SUCC(alarm(TEST_TIMEOUT_SECONDS));

	int pipefds[2];
	TEST_SUCC(pipe(pipefds));

	struct pollfd pfd = {
		.fd = pipefds[0],
		.events = POLLIN,
	};
	TEST_SUCC(timer_settime(signal_timer, 0, &periodic_timer, NULL));
	TEST(poll(&pfd, 1, -1), EINTR, _ret == -1 && signal_count > 0);
	TEST_SUCC(timer_settime(signal_timer, 0, &stopped_timer, NULL));

	TEST_SUCC(close(pipefds[0]));
	TEST_SUCC(close(pipefds[1]));
	TEST_SUCC(alarm(0));
}
END_TEST()

FN_TEST(futex_wait_without_timeout_restarts)
{
	signal_count = 0;
	TEST_SUCC(alarm(TEST_TIMEOUT_SECONDS));

	uint32_t *futex_word =
		TEST_RES(mmap(NULL, sizeof(*futex_word), PROT_READ | PROT_WRITE,
			      MAP_SHARED | MAP_ANONYMOUS, -1, 0),
			 _ret != MAP_FAILED);
	*futex_word = 0;

	pid_t parent = getpid();
	pid_t child = TEST_SUCC(fork());
	if (child == 0) {
		CHECK(alarm(TEST_TIMEOUT_SECONDS));
		sleep(1);
		CHECK(kill(parent, SIGUSR1));
		sleep(1);
		// Keep the value unchanged and wait for the parent to reenter FUTEX_WAIT.
		while (CHECK(syscall(SYS_futex, futex_word, FUTEX_WAKE, 1, NULL,
				     NULL, 0)) == 0)
			CHECK(usleep(SIGNAL_INTERVAL_NS / 1000));
		_exit(0);
	}

	TEST_RES(syscall(SYS_futex, futex_word, FUTEX_WAIT, 0, NULL, NULL, 0),
		 _ret == 0 && signal_count > 0);

	int status = 0;
	TEST_RES(waitpid(child, &status, 0), _ret == child &&
						     WIFEXITED(status) &&
						     WEXITSTATUS(status) == 0);
	TEST_SUCC(munmap(futex_word, sizeof(*futex_word)));
	TEST_SUCC(alarm(0));
}
END_TEST()

FN_TEST(futex_wait_with_timeout_does_not_restart)
{
	for (int wait_bitset = 0; wait_bitset <= 1; wait_bitset++) {
		signal_count = 0;
		TEST_SUCC(alarm(TEST_TIMEOUT_SECONDS));
		uint32_t futex_word = 0;
		struct timespec timeout;
		if (wait_bitset) {
			TEST_SUCC(clock_gettime(CLOCK_MONOTONIC, &timeout));
			timeout.tv_sec += 3;
		} else {
			timeout.tv_sec = 3;
			timeout.tv_nsec = 0;
		}

		int futex_op = wait_bitset ? FUTEX_WAIT_BITSET_PRIVATE :
					     FUTEX_WAIT_PRIVATE;
		uint32_t bitset = wait_bitset ? FUTEX_BITSET_MATCH_ANY : 0;
		TEST_SUCC(
			timer_settime(signal_timer, 0, &periodic_timer, NULL));
		TEST(syscall(SYS_futex, &futex_word, futex_op, 0, &timeout,
			     NULL, bitset),
		     EINTR, _ret == -1 && signal_count > 0);
		TEST_SUCC(timer_settime(signal_timer, 0, &stopped_timer, NULL));
		TEST_SUCC(alarm(0));
	}
}
END_TEST()

FN_TEST(socket_read_without_timeout_restarts)
{
	signal_count = 0;
	TEST_SUCC(alarm(TEST_TIMEOUT_SECONDS));

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
	TEST_SUCC(alarm(0));
}
END_TEST()

FN_TEST(socket_read_with_timeout_does_not_restart)
{
	signal_count = 0;
	TEST_SUCC(alarm(TEST_TIMEOUT_SECONDS));

	int sockets[2];
	TEST_SUCC(socketpair(AF_UNIX, SOCK_STREAM, 0, sockets));

	struct timeval timeout = { .tv_sec = 5 };
	TEST_SUCC(setsockopt(sockets[0], SOL_SOCKET, SO_RCVTIMEO, &timeout,
			     sizeof(timeout)));

	char byte = 0;
	TEST_SUCC(timer_settime(signal_timer, 0, &periodic_timer, NULL));
	TEST(read(sockets[0], &byte, sizeof(byte)), EINTR,
	     _ret == -1 && signal_count > 0);
	TEST_SUCC(timer_settime(signal_timer, 0, &stopped_timer, NULL));

	TEST_SUCC(close(sockets[0]));
	TEST_SUCC(close(sockets[1]));
	TEST_SUCC(alarm(0));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(timer_delete(signal_timer));
}
END_SETUP()
