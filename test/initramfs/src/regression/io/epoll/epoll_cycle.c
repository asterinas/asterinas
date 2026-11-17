// SPDX-License-Identifier: MPL-2.0

#include "../../common/test.h"
#include <sys/epoll.h>
#include <unistd.h>

// An epoll file cannot be used as the target of any epoll_ctl command,
// whether it is referred to by the same fd or by a duplicated one.
FN_TEST(epoll_self_disallowed)
{
	struct epoll_event ev = { .events = EPOLLIN, .data.u64 = 0 };
	int epfd, dup_epfd;

	epfd = TEST_SUCC(epoll_create1(0));
	dup_epfd = TEST_SUCC(dup(epfd));

	TEST_ERRNO(epoll_ctl(epfd, EPOLL_CTL_ADD, epfd, &ev), EINVAL);
	TEST_ERRNO(epoll_ctl(epfd, EPOLL_CTL_ADD, dup_epfd, &ev), EINVAL);
	TEST_ERRNO(epoll_ctl(dup_epfd, EPOLL_CTL_ADD, epfd, &ev), EINVAL);

	TEST_ERRNO(epoll_ctl(epfd, EPOLL_CTL_DEL, epfd, NULL), EINVAL);
	TEST_ERRNO(epoll_ctl(epfd, EPOLL_CTL_DEL, dup_epfd, NULL), EINVAL);
	TEST_ERRNO(epoll_ctl(dup_epfd, EPOLL_CTL_DEL, epfd, NULL), EINVAL);

	TEST_ERRNO(epoll_ctl(epfd, EPOLL_CTL_MOD, epfd, &ev), EINVAL);
	TEST_ERRNO(epoll_ctl(epfd, EPOLL_CTL_MOD, dup_epfd, &ev), EINVAL);
	TEST_ERRNO(epoll_ctl(dup_epfd, EPOLL_CTL_MOD, epfd, &ev), EINVAL);

	TEST_SUCC(close(epfd));
	TEST_SUCC(close(dup_epfd));
}
END_TEST()
