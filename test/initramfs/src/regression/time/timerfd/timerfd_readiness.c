// SPDX-License-Identifier: MPL-2.0

#include <stdint.h>
#include <sys/epoll.h>
#include <sys/timerfd.h>
#include <time.h>
#include <unistd.h>

#include "../../common/test.h"

static int tfd;
static int epfd;
static struct epoll_event ev;

static const struct itimerspec ten_ms = {
	.it_value = { .tv_sec = 0, .tv_nsec = 10 * 1000 * 1000 },
};
static const struct itimerspec disarmed = { 0 };

FN_SETUP(create)
{
	tfd = CHECK(timerfd_create(CLOCK_MONOTONIC, TFD_NONBLOCK));
	epfd = CHECK(epoll_create1(0));

	struct epoll_event add = { .events = EPOLLIN, .data.fd = tfd };
	CHECK(epoll_ctl(epfd, EPOLL_CTL_ADD, tfd, &add));
}
END_SETUP()

FN_TEST(disarm_clears_readiness)
{
	uint64_t ticks;

	TEST_SUCC(timerfd_settime(tfd, 0, &ten_ms, NULL));
	TEST_RES(epoll_wait(epfd, &ev, 1, 1000), _ret == 1);

	// Disarming resets the expiration count, so the fd is no longer readable.
	TEST_SUCC(timerfd_settime(tfd, 0, &disarmed, NULL));
	TEST_RES(epoll_wait(epfd, &ev, 1, 0), _ret == 0);
	TEST_ERRNO(read(tfd, &ticks, sizeof(ticks)), EAGAIN);
}
END_TEST()

FN_TEST(read_clears_readiness)
{
	uint64_t ticks;

	TEST_SUCC(timerfd_settime(tfd, 0, &ten_ms, NULL));
	TEST_RES(epoll_wait(epfd, &ev, 1, 1000), _ret == 1);

	TEST_RES(read(tfd, &ticks, sizeof(ticks)),
		 _ret == sizeof(ticks) && ticks == 1);
	TEST_RES(epoll_wait(epfd, &ev, 1, 0), _ret == 0);
}
END_TEST()

FN_TEST(rearm_restores_readiness)
{
	uint64_t ticks;

	TEST_SUCC(timerfd_settime(tfd, 0, &ten_ms, NULL));
	TEST_RES(epoll_wait(epfd, &ev, 1, 1000), _ret == 1);

	// Rearming also resets the count; the fd becomes readable again only
	// when the new timeout expires.
	TEST_SUCC(timerfd_settime(tfd, 0, &ten_ms, NULL));
	TEST_RES(epoll_wait(epfd, &ev, 1, 0), _ret == 0);
	TEST_RES(epoll_wait(epfd, &ev, 1, 1000), _ret == 1);
	TEST_RES(read(tfd, &ticks, sizeof(ticks)),
		 _ret == sizeof(ticks) && ticks == 1);
}
END_TEST()

FN_TEST(cleanup)
{
	TEST_SUCC(close(epfd));
	TEST_SUCC(close(tfd));
}
END_TEST()
