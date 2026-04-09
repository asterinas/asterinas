// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include "../../common/test.h"
#include <signal.h>
#include <string.h>
#include <sys/poll.h>
#include <sys/syscall.h>
#include <sys/uio.h>
#include <unistd.h>

FN_SETUP()
{
	signal(SIGPIPE, SIG_IGN);
}
END_SETUP()

FN_TEST(close_without_data_then_read)
{
	int fildes[2];
	char buf[8] = { 0 };

	TEST_SUCC(pipe(fildes));

	TEST_SUCC(close(fildes[1]));

	TEST_RES(read(fildes[0], buf, sizeof(buf)), _ret == 0);
	TEST_ERRNO(write(fildes[0], buf, sizeof(buf)), EBADF);

	TEST_RES(read(fildes[0], buf, 0), _ret == 0);
	TEST_ERRNO(write(fildes[0], buf, 0), EBADF);

	TEST_SUCC(close(fildes[0]));
}
END_TEST()

FN_TEST(close_without_data_then_write)
{
	int fildes[2];
	char buf[8] = { 0 };

	TEST_SUCC(pipe(fildes));

	TEST_SUCC(close(fildes[0]));

	TEST_ERRNO(read(fildes[1], buf, sizeof(buf)), EBADF);
	TEST_ERRNO(write(fildes[1], buf, sizeof(buf)), EPIPE);

	TEST_ERRNO(read(fildes[1], buf, 0), EBADF);
	TEST_RES(write(fildes[1], buf, 0), _ret == 0);

	TEST_SUCC(close(fildes[1]));
}
END_TEST()

FN_TEST(close_with_data_then_read)
{
	int fildes[2];
	char buf[8] = { 0 };

	TEST_SUCC(pipe(fildes));

	TEST_RES(write(fildes[1], "hello", 5), _ret == 5);
	TEST_SUCC(close(fildes[1]));

	TEST_RES(read(fildes[0], buf, 2),
		 _ret == 2 && strncmp(buf, "he", 2) == 0);
	TEST_RES(read(fildes[0], buf, sizeof(buf)),
		 _ret == 3 && strncmp(buf, "llo", 3) == 0);

	TEST_RES(read(fildes[0], buf, sizeof(buf)), _ret == 0);
	TEST_ERRNO(write(fildes[0], buf, sizeof(buf)), EBADF);

	TEST_RES(read(fildes[0], buf, 0), _ret == 0);
	TEST_ERRNO(write(fildes[0], buf, 0), EBADF);

	TEST_SUCC(close(fildes[0]));
}
END_TEST()

FN_TEST(close_with_data_then_write)
{
	int fildes[2];
	char buf[8] = { 0 };

	TEST_SUCC(pipe(fildes));

	TEST_RES(write(fildes[1], "hello", 5), _ret == 5);
	TEST_SUCC(close(fildes[0]));

	TEST_ERRNO(read(fildes[1], buf, sizeof(buf)), EBADF);
	TEST_ERRNO(write(fildes[1], buf, sizeof(buf)), EPIPE);

	TEST_ERRNO(read(fildes[1], buf, 0), EBADF);
	TEST_RES(write(fildes[1], buf, 0), _ret == 0);

	TEST_SUCC(close(fildes[1]));
}
END_TEST()

#define POLL_MASK (POLLIN | POLLOUT | POLLHUP | POLLERR)

FN_TEST(poll_basic)
{
	int fildes[2];
	char buf[8];
	struct pollfd pfd = { .events = POLL_MASK };

	TEST_SUCC(pipe(fildes));

	pfd.fd = fildes[0];
	TEST_RES(poll(&pfd, 1, 0), (pfd.revents & POLL_MASK) == 0);

	pfd.fd = fildes[1];
	TEST_RES(poll(&pfd, 1, 0), (pfd.revents & POLL_MASK) == POLLOUT);

	TEST_RES(write(fildes[1], "hello", 5), _ret == 5);

	pfd.fd = fildes[0];
	TEST_RES(poll(&pfd, 1, 0), (pfd.revents & POLL_MASK) == POLLIN);

	pfd.fd = fildes[1];
	TEST_RES(poll(&pfd, 1, 0), (pfd.revents & POLL_MASK) == POLLOUT);

	TEST_RES(read(fildes[0], buf, sizeof(buf)), _ret == 5);

	pfd.fd = fildes[0];
	TEST_RES(poll(&pfd, 1, 0), (pfd.revents & POLL_MASK) == 0);

	pfd.fd = fildes[1];
	TEST_RES(poll(&pfd, 1, 0), (pfd.revents & POLL_MASK) == POLLOUT);

	TEST_SUCC(close(fildes[0]));
	TEST_SUCC(close(fildes[1]));
}
END_TEST()

FN_TEST(close_first_then_poll)
{
	int fildes[2];
	struct pollfd pfd = { .events = POLLIN | POLLOUT };

	TEST_SUCC(pipe(fildes));

	TEST_RES(write(fildes[1], "hello", 5), _ret == 5);
	TEST_SUCC(close(fildes[0]));

	pfd.fd = fildes[1];
	TEST_RES(poll(&pfd, 1, 0),
		 (pfd.revents & POLL_MASK) == (POLLOUT | POLLERR));

	TEST_SUCC(close(fildes[1]));
}
END_TEST()

FN_TEST(close_second_then_poll)
{
	int fildes[2];
	char buf[8];
	struct pollfd pfd = { .events = POLLIN | POLLOUT };

	TEST_SUCC(pipe(fildes));

	TEST_RES(write(fildes[1], "hello", 5), _ret == 5);
	TEST_SUCC(close(fildes[1]));

	pfd.fd = fildes[0];
	TEST_RES(poll(&pfd, 1, 0),
		 (pfd.revents & POLL_MASK) == (POLLIN | POLLHUP));

	TEST_RES(read(fildes[0], buf, sizeof(buf)),
		 _ret == 5 && strncmp(buf, "hello", 5) == 0);

	pfd.fd = fildes[0];
	TEST_RES(poll(&pfd, 1, 0), (pfd.revents & POLL_MASK) == POLLHUP);

	TEST_SUCC(close(fildes[0]));
}
END_TEST()

// See also `zero_recvs_may_fail` in `unix_err.c`
FN_TEST(zero_reads_always_succeed)
{
	int fildes[2];
	char buf[1] = { 'z' };

	TEST_SUCC(pipe(fildes));

	TEST_SUCC(read(fildes[0], buf, 0));

	TEST_RES(write(fildes[1], buf, 1), _ret == 1);
	TEST_SUCC(read(fildes[0], buf, 0));

	TEST_SUCC(close(fildes[0]));
}
END_TEST()

// See also `zero_sends_may_fail` in `unix_err.c`
FN_TEST(zero_writes_always_succeed)
{
	int fildes[2];
	char buf[1] = { 'z' };

	TEST_SUCC(pipe(fildes));

	TEST_SUCC(write(fildes[1], buf, 0));

	TEST_SUCC(close(fildes[0]));
	TEST_SUCC(write(fildes[1], buf, 0));

	TEST_SUCC(close(fildes[1]));
}
END_TEST()

// Verifies the Linux-compatible `ESPIPE` result for positional I/O on pipes.
FN_TEST(zero_length_positional_io_fails_on_pipe)
{
	int fildes[2];
	char buf[1] = { 'z' };
	struct iovec iov = { .iov_base = buf, .iov_len = 0 };

	TEST_SUCC(pipe(fildes));

	TEST_ERRNO(syscall(SYS_pread64, fildes[0], buf, 0, 3), ESPIPE);
	TEST_ERRNO(syscall(SYS_pread64, fildes[1], buf, 0, 3), ESPIPE);
	TEST_ERRNO(syscall(SYS_pwrite64, fildes[1], buf, 0, 3), ESPIPE);
	TEST_ERRNO(syscall(SYS_pwrite64, fildes[0], buf, 0, 3), ESPIPE);
	TEST_ERRNO(syscall(SYS_preadv, fildes[0], NULL, 0, 3, 0), ESPIPE);
	TEST_ERRNO(syscall(SYS_preadv, fildes[1], NULL, 0, 3, 0), ESPIPE);
	TEST_ERRNO(syscall(SYS_pwritev, fildes[1], NULL, 0, 3, 0), ESPIPE);
	TEST_ERRNO(syscall(SYS_pwritev, fildes[0], NULL, 0, 3, 0), ESPIPE);
	TEST_ERRNO(syscall(SYS_preadv, fildes[0], &iov, 1, 3, 0), ESPIPE);
	TEST_ERRNO(syscall(SYS_preadv, fildes[1], &iov, 1, 3, 0), ESPIPE);
	TEST_ERRNO(syscall(SYS_pwritev, fildes[1], &iov, 1, 3, 0), ESPIPE);
	TEST_ERRNO(syscall(SYS_pwritev, fildes[0], &iov, 1, 3, 0), ESPIPE);

	TEST_SUCC(close(fildes[0]));
	TEST_SUCC(close(fildes[1]));
}
END_TEST()
