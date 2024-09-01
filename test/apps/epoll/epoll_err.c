// SPDX-License-Identifier: MPL-2.0

#include "../network/test.h"
#include <unistd.h>
#include <sys/epoll.h>

FN_TEST(epoll_add_del)
{
	int fildes[2];
	int epfd, rfd, wfd, rfd2;
	struct epoll_event ev;

	// Setup pipes
	TEST_SUCC(pipe(fildes));
	rfd = fildes[0];
	wfd = fildes[1];
	TEST_SUCC(write(wfd, "", 1));

	// Setup epoll
	epfd = TEST_SUCC(epoll_create1(0));
	ev.events = EPOLLIN;
	ev.data.fd = rfd;
	TEST_SUCC(epoll_ctl(epfd, EPOLL_CTL_ADD, rfd, &ev));

	// Dup and close
	rfd2 = dup(rfd);
	close(rfd);

	// No way to operate on closed file descriptors
	TEST_RES(epoll_wait(epfd, &ev, 1, 0), _ret == 1 && ev.data.fd == rfd);
	TEST_ERRNO(epoll_ctl(epfd, EPOLL_CTL_DEL, rfd, NULL), EBADF);
	TEST_ERRNO(epoll_ctl(epfd, EPOLL_CTL_DEL, rfd2, NULL), ENOENT);

	// Old file descriptor and new file
	TEST_RES(pipe(fildes), fildes[0] == rfd);
	TEST_ERRNO(epoll_ctl(epfd, EPOLL_CTL_DEL, rfd, NULL), ENOENT);
	TEST_SUCC(epoll_ctl(epfd, EPOLL_CTL_ADD, rfd, &ev));
	TEST_SUCC(epoll_ctl(epfd, EPOLL_CTL_DEL, rfd, NULL));
	TEST_SUCC(close(fildes[0]));
	TEST_SUCC(close(fildes[1]));

	// Old file descriptor and old file
	TEST_RES(dup(rfd2), _ret == rfd);
	TEST_SUCC(epoll_ctl(epfd, EPOLL_CTL_DEL, rfd, NULL));

	// Clean up
	TEST_SUCC(close(epfd));
	TEST_SUCC(close(rfd));
	TEST_SUCC(close(wfd));
	TEST_SUCC(close(rfd2));
}
END_TEST()
