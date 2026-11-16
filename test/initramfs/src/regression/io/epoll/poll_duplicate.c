// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include "../../common/test.h"
#include <poll.h>
#include <sys/socket.h>
#include <time.h>
#include <unistd.h>

static void *write_delayed(void *arg)
{
	int fd = *(int *)arg;

	// Let the polling thread register its wait before the data arrives.
	CHECK(usleep(100000));
	CHECK_WITH(write(fd, "x", 1), _ret == 1);
	return NULL;
}

static int wait_for_events(struct pollfd *fds, int use_ppoll)
{
	if (use_ppoll) {
		struct timespec timeout = { .tv_sec = 2 };

		return ppoll(fds, 2, &timeout, NULL);
	}

	return poll(fds, 2, 2000);
}

FN_TEST(duplicate_fd_wakeup)
{
	const short masks[][2] = {
		{ POLLIN, 0 },	     { 0, POLLIN },	 { POLLIN, POLLPRI },
		{ POLLPRI, POLLIN }, { POLLIN, POLLIN },
	};

	for (int use_ppoll = 0; use_ppoll < 2; ++use_ppoll) {
		for (int use_dup = 0; use_dup < 2; ++use_dup) {
			for (size_t i = 0; i < sizeof(masks) / sizeof(masks[0]);
			     ++i) {
				int sockets[2];
				TEST_SUCC(socketpair(AF_UNIX, SOCK_STREAM, 0,
						     sockets));

				int fd = use_dup ? TEST_SUCC(dup(sockets[0])) :
						   sockets[0];
				struct pollfd fds[] = {
					{ .fd = sockets[0],
					  .events = masks[i][0] },
					{ .fd = fd, .events = masks[i][1] },
				};

				pthread_t writer;
				TEST_SUCC(pthread_create(&writer, NULL,
							 write_delayed,
							 &sockets[1]));
				int num_ready = !!(masks[i][0] & POLLIN) +
						!!(masks[i][1] & POLLIN);
				TEST_RES(wait_for_events(fds, use_ppoll),
					 _ret == num_ready);
				TEST_RES(fds[0].revents,
					 _ret == (masks[i][0] & POLLIN));
				TEST_RES(fds[1].revents,
					 _ret == (masks[i][1] & POLLIN));
				TEST_SUCC(pthread_join(writer, NULL));

				if (use_dup)
					TEST_SUCC(close(fd));
				TEST_SUCC(close(sockets[0]));
				TEST_SUCC(close(sockets[1]));
			}
		}
	}
}
END_TEST()

FN_TEST(duplicate_fd_hangup)
{
	for (int use_ppoll = 0; use_ppoll < 2; ++use_ppoll) {
		int fds[2];
		TEST_SUCC(pipe(fds));
		TEST_SUCC(close(fds[1]));

		struct pollfd poll_fds[] = {
			{ .fd = fds[0], .events = POLLIN },
			{ .fd = fds[0], .events = 0 },
		};
		TEST_RES(wait_for_events(poll_fds, use_ppoll), _ret == 2);
		TEST_RES(poll_fds[0].revents, _ret == POLLHUP);
		TEST_RES(poll_fds[1].revents, _ret == POLLHUP);

		TEST_SUCC(close(fds[0]));
	}
}
END_TEST()
