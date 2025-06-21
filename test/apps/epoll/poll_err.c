// SPDX-License-Identifier: MPL-2.0

#include "../test.h"
#include <unistd.h>
#include <sys/poll.h>

FN_TEST(poll_nval)
{
	int fildes[2];
	int rfd, wfd;
	struct pollfd fds[3];

	TEST_SUCC(pipe(fildes));
	rfd = fildes[0];
	wfd = fildes[1];
	TEST_SUCC(write(wfd, "", 1));

	fds[0].fd = rfd;
	fds[1].fd = 1000;
	fds[2].fd = wfd;

	fds[0].events = POLLIN | POLLOUT;
	fds[1].events = POLLIN | POLLOUT;
	fds[2].events = POLLIN | POLLOUT;

	TEST_RES(poll(fds, 3, 0), _ret == 3 && fds[0].revents == POLLIN &&
					  fds[1].revents == POLLNVAL &&
					  fds[2].revents == POLLOUT);

	TEST_SUCC(close(rfd));
	TEST_SUCC(close(wfd));
}
END_TEST()

FN_TEST(select_bafd)
{
	fd_set rfds;

	FD_ZERO(&rfds);
	FD_SET(100, &rfds);

	TEST_ERRNO(select(200, &rfds, NULL, NULL, NULL), EBADF);
}
END_TEST()
