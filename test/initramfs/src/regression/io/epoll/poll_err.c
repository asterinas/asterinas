// SPDX-License-Identifier: MPL-2.0

#include "../../common/test.h"
#include <signal.h>
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

	// FIXME: Linux will "ignore any file descriptor in these sets that
	// is greater than the maximum file descriptor number that the
	// process currently has open." See the BUGS section in
	// <https://man7.org/linux/man-pages/man2/select.2.html>.
	// But Asterinas will always report `EBADF` in this case.
#ifdef __asterinas__
	TEST_ERRNO(select(200, &rfds, NULL, NULL, NULL), EBADF);
#endif
}
END_TEST()

static void on_sigalrm(int sig)
{
	// Do nothing.
}

FN_SETUP(set_sigalrm)
{
	CHECK(signal(SIGALRM, &on_sigalrm));
}
END_SETUP()

FN_TEST(poll_eintr)
{
	int fildes[2];
	int rfd, wfd;
	struct pollfd fds[2];

	TEST_SUCC(pipe(fildes));
	rfd = fildes[0];
	wfd = fildes[1];

	fds[0].fd = rfd;
	fds[0].events = POLLIN;
	fds[0].revents = POLLIN;

	// A negative FD indicates an invalid entry which the kernel should ignore.
	fds[1].fd = -rfd;
	fds[1].events = POLLIN;
	fds[1].revents = POLLIN;

	// Do a `poll` syscall that will be interrupted by SIGALRM.
	TEST_SUCC(alarm(1));
	TEST_ERRNO(poll(fds, 2, 2000), EINTR);

	// Even if `poll` fails with `EINTR`, `revents` must be cleared.
	TEST_RES(fds[0].revents, _ret == 0);
	TEST_RES(fds[1].revents, _ret == 0);
	// However, the FD should not be altered for either valid or invalid entries.
	TEST_RES(fds[0].fd, _ret == rfd);
	TEST_RES(fds[1].fd, _ret == -rfd);

	TEST_SUCC(close(rfd));
	TEST_SUCC(close(wfd));
}
END_TEST()

FN_TEST(select_eintr)
{
	int fildes[2];
	int rfd, wfd;
	fd_set rfds;

	TEST_SUCC(pipe(fildes));
	rfd = fildes[0];
	wfd = fildes[1];

	FD_ZERO(&rfds);
	FD_SET(rfd, &rfds);

	// Do a `select` syscall that will be interrupted by SIGALRM.
	TEST_SUCC(alarm(1));
	TEST_ERRNO(select(rfd + 1, &rfds, NULL, NULL, NULL), EINTR);

	// If `select` fails with `EINTR`, `rfds` will not be cleared.
	TEST_RES(FD_ISSET(rfd, &rfds), _ret);

	TEST_SUCC(close(rfd));
	TEST_SUCC(close(wfd));
}
END_TEST()
