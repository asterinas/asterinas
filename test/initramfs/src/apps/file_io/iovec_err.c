// SPDX-License-Identifier: MPL-2.0

#include <stdint.h>
#include <sys/fcntl.h>
#include <sys/socket.h>
#include <sys/uio.h>
#include <unistd.h>

#include "../test.h"

static char buf[16];
static struct iovec iov_long[UIO_MAXIOV + 2];
static struct iovec iov_inv[2];

FN_SETUP(iov)
{
	int i;

	for (i = 0; i <= UIO_MAXIOV + 1; ++i)
		iov_long[i] = (struct iovec){ .iov_base = buf,
					      .iov_len = sizeof(buf) };

	iov_inv[0] = (struct iovec){ .iov_base = buf, .iov_len = sizeof(buf) };
	iov_inv[1] = (struct iovec){ .iov_base = buf, .iov_len = -1234 };
}
END_SETUP()

FN_TEST(readv)
{
	int fd;

	fd = TEST_SUCC(open("/dev/zero", O_RDONLY));

	TEST_SUCC(readv(fd, iov_long, UIO_MAXIOV - 1));
	TEST_SUCC(readv(fd, iov_long, UIO_MAXIOV));
	TEST_ERRNO(readv(fd, iov_long, UIO_MAXIOV + 1), EINVAL);

	TEST_SUCC(readv(fd, iov_inv, 1));
	TEST_ERRNO(readv(fd, iov_inv, 2), EINVAL);

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(writev)
{
	int fd;

	fd = TEST_SUCC(open("/dev/null", O_WRONLY));

	TEST_SUCC(writev(fd, iov_long, UIO_MAXIOV - 1));
	TEST_SUCC(writev(fd, iov_long, UIO_MAXIOV));
	TEST_ERRNO(writev(fd, iov_long, UIO_MAXIOV + 1), EINVAL);

	TEST_SUCC(writev(fd, iov_inv, 1));
	TEST_ERRNO(writev(fd, iov_inv, 2), EINVAL);

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(sendmsg_recvmsg)
{
	int fds[2];
	struct msghdr msgh;

	TEST_SUCC(socketpair(AF_UNIX, SOCK_SEQPACKET, 0, fds));

	memset(&msgh, 0, sizeof(msgh));
#define sendv(fd, iov, iovcnt)            \
	({                                \
		msgh.msg_iov = iov;       \
		msgh.msg_iovlen = iovcnt; \
		sendmsg(fd, &msgh, 0);    \
	})
#define recvv(fd, iov, iovcnt)            \
	({                                \
		msgh.msg_iov = iov;       \
		msgh.msg_iovlen = iovcnt; \
		recvmsg(fd, &msgh, 0);    \
	})

	TEST_SUCC(sendv(fds[0], iov_long, UIO_MAXIOV - 1));
	TEST_SUCC(sendv(fds[0], iov_long, UIO_MAXIOV));
	TEST_ERRNO(sendv(fds[0], iov_long, UIO_MAXIOV + 1), EMSGSIZE);

	TEST_SUCC(recvv(fds[1], iov_long, UIO_MAXIOV - 1));
	TEST_SUCC(recvv(fds[1], iov_long, UIO_MAXIOV));
	TEST_ERRNO(recvv(fds[1], iov_long, UIO_MAXIOV + 1), EMSGSIZE);

	TEST_SUCC(sendv(fds[0], iov_inv, 1));
	TEST_ERRNO(sendv(fds[0], iov_inv, 2), EINVAL);

	TEST_SUCC(recvv(fds[1], iov_inv, 1));
	TEST_ERRNO(recvv(fds[1], iov_inv, 2), EINVAL);

#undef sendv
#undef recvv

	TEST_SUCC(close(fds[0]));
	TEST_SUCC(close(fds[1]));
}
END_TEST()
