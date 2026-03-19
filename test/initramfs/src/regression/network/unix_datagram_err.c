// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <unistd.h>
#include <stddef.h>
#include <sys/poll.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <sys/wait.h>

#include "../common/test.h"

static int sk_unbound;
static int sk_bound;
static int sk_connected;

#define UNIX_ADDR(path) \
	((struct sockaddr_un){ .sun_family = AF_UNIX, .sun_path = path })

#define PATH_OFFSET offsetof(struct sockaddr_un, sun_path)

#define UNNAMED_ADDR UNIX_ADDR("")
#define UNNAMED_ADDRLEN PATH_OFFSET

#define BOUND_ADDR UNIX_ADDR("//tmp/B0")
#define BOUND_ADDRLEN (PATH_OFFSET + 9)

FN_SETUP(unbound)
{
	sk_unbound = CHECK(socket(PF_UNIX, SOCK_DGRAM | SOCK_NONBLOCK, 0));
}
END_SETUP()

FN_SETUP(bound)
{
	sk_bound = CHECK(socket(PF_UNIX, SOCK_DGRAM | SOCK_NONBLOCK, 0));

	CHECK(bind(sk_bound, (struct sockaddr *)&BOUND_ADDR, BOUND_ADDRLEN));
}
END_SETUP()

FN_SETUP(connected)
{
	sk_connected = CHECK(socket(PF_UNIX, SOCK_DGRAM | SOCK_NONBLOCK, 0));

	CHECK(connect(sk_connected, (struct sockaddr *)&BOUND_ADDR,
		      BOUND_ADDRLEN));
}
END_SETUP()

FN_TEST(getsockname)
{
	struct sockaddr_un addr;
	socklen_t addrlen;

	addrlen = sizeof(addr);
	TEST_RES(getsockname(sk_unbound, (struct sockaddr *)&addr, &addrlen),
		 addrlen == UNNAMED_ADDRLEN &&
			 memcmp(&addr, &UNNAMED_ADDR, UNNAMED_ADDRLEN) == 0);

	addrlen = sizeof(addr);
	TEST_RES(getsockname(sk_bound, (struct sockaddr *)&addr, &addrlen),
		 addrlen == BOUND_ADDRLEN &&
			 memcmp(&addr, &BOUND_ADDR, BOUND_ADDRLEN) == 0);

	addrlen = sizeof(addr);
	TEST_RES(getsockname(sk_connected, (struct sockaddr *)&addr, &addrlen),
		 addrlen == UNNAMED_ADDRLEN &&
			 memcmp(&addr, &UNNAMED_ADDR, UNNAMED_ADDRLEN) == 0);
}
END_TEST()

FN_TEST(getpeername)
{
	struct sockaddr_un addr;
	socklen_t addrlen;

	addrlen = sizeof(addr);
	TEST_ERRNO(getpeername(sk_unbound, (struct sockaddr *)&addr, &addrlen),
		   ENOTCONN);

	addrlen = sizeof(addr);
	TEST_ERRNO(getpeername(sk_bound, (struct sockaddr *)&addr, &addrlen),
		   ENOTCONN);

	addrlen = sizeof(addr);
	TEST_RES(getpeername(sk_connected, (struct sockaddr *)&addr, &addrlen),
		 addrlen == BOUND_ADDRLEN &&
			 memcmp(&addr, &BOUND_ADDR, BOUND_ADDRLEN) == 0);
}
END_TEST()

FN_TEST(bind)
{
	TEST_ERRNO(bind(sk_bound, (struct sockaddr *)&UNIX_ADDR("\0Z"),
			PATH_OFFSET + 1),
		   EINVAL);

	TEST_SUCC(bind(sk_bound, (struct sockaddr *)&UNNAMED_ADDR,
		       UNNAMED_ADDRLEN));
}
END_TEST()

FN_TEST(bind_connected)
{
	int fildes[2], sk;
	struct sockaddr_un addr;
	socklen_t addrlen;

	TEST_SUCC(socketpair(PF_UNIX, SOCK_DGRAM, 0, fildes));
	sk = TEST_SUCC(socket(PF_UNIX, SOCK_DGRAM, 0));

	TEST_SUCC(bind(fildes[0], (struct sockaddr *)&UNIX_ADDR("\0X"),
		       PATH_OFFSET + 2));
	addrlen = sizeof(addr);
	TEST_RES(getpeername(fildes[1], (struct sockaddr *)&addr, &addrlen),
		 addrlen == PATH_OFFSET + 2 && memcmp(&addr, &UNIX_ADDR("\0X"),
						      PATH_OFFSET + 2) == 0);

	TEST_SUCC(bind(fildes[1], (struct sockaddr *)&UNIX_ADDR("\0Y"),
		       PATH_OFFSET + 2));
	addrlen = sizeof(addr);
	TEST_RES(getpeername(fildes[0], (struct sockaddr *)&addr, &addrlen),
		 addrlen == PATH_OFFSET + 2 && memcmp(&addr, &UNIX_ADDR("\0Y"),
						      PATH_OFFSET + 2) == 0);

	TEST_ERRNO(bind(fildes[0], (struct sockaddr *)&UNIX_ADDR("\0Z"),
			PATH_OFFSET + 2),
		   EINVAL);
	TEST_ERRNO(bind(fildes[1], (struct sockaddr *)&UNIX_ADDR("\0Z"),
			PATH_OFFSET + 2),
		   EINVAL);
	TEST_SUCC(bind(fildes[0], (struct sockaddr *)&UNNAMED_ADDR,
		       UNNAMED_ADDRLEN));
	TEST_SUCC(bind(fildes[1], (struct sockaddr *)&UNNAMED_ADDR,
		       UNNAMED_ADDRLEN));

	// Closing the socket will release the bound address.
	// So another socket can bind to it again.
	TEST_ERRNO(bind(sk, (struct sockaddr *)&UNIX_ADDR("\0X"),
			PATH_OFFSET + 2),
		   EADDRINUSE);
	TEST_SUCC(close(fildes[0]));
	TEST_SUCC(bind(sk, (struct sockaddr *)&UNIX_ADDR("\0X"),
		       PATH_OFFSET + 2));

	// But the released address is still "visible" from
	// the previously connected socket.
	addrlen = sizeof(addr);
	TEST_RES(getpeername(fildes[1], (struct sockaddr *)&addr, &addrlen),
		 addrlen == PATH_OFFSET + 2 && memcmp(&addr, &UNIX_ADDR("\0X"),
						      PATH_OFFSET + 2) == 0);

	TEST_SUCC(close(fildes[1]));
	TEST_SUCC(close(sk));
}
END_TEST()

FN_TEST(connect)
{
	TEST_ERRNO(connect(sk_unbound, (struct sockaddr *)&UNIX_ADDR("\0X"),
			   PATH_OFFSET + 2),
		   ECONNREFUSED);

	TEST_ERRNO(connect(sk_bound, (struct sockaddr *)&UNIX_ADDR("\0X"),
			   PATH_OFFSET + 2),
		   ECONNREFUSED);

	TEST_SUCC(connect(sk_connected, (struct sockaddr *)&BOUND_ADDR,
			  BOUND_ADDRLEN));

	TEST_ERRNO(connect(sk_connected, (struct sockaddr *)&UNIX_ADDR("\0X"),
			   PATH_OFFSET + 2),
		   ECONNREFUSED);
}
END_TEST()

FN_TEST(listen)
{
	TEST_ERRNO(listen(sk_unbound, 10), EOPNOTSUPP);

	TEST_ERRNO(listen(sk_bound, 10), EOPNOTSUPP);

	TEST_ERRNO(listen(sk_connected, 10), EOPNOTSUPP);
}
END_TEST()

FN_TEST(accept)
{
	TEST_ERRNO(accept(sk_unbound, NULL, NULL), EOPNOTSUPP);

	TEST_ERRNO(accept(sk_bound, NULL, NULL), EOPNOTSUPP);

	TEST_ERRNO(accept(sk_connected, NULL, NULL), EOPNOTSUPP);
}
END_TEST()

FN_TEST(send)
{
	char buf[1] = { 'z' };

	TEST_ERRNO(send(sk_unbound, buf, 1, 0), ENOTCONN);
	TEST_ERRNO(send(sk_unbound, buf, 0, 0), ENOTCONN);
	TEST_ERRNO(write(sk_unbound, buf, 1), ENOTCONN);
	TEST_ERRNO(write(sk_unbound, buf, 0), ENOTCONN);

	TEST_ERRNO(send(sk_bound, buf, 1, 0), ENOTCONN);
	TEST_ERRNO(send(sk_bound, buf, 0, 0), ENOTCONN);
	TEST_ERRNO(write(sk_bound, buf, 1), ENOTCONN);
	TEST_ERRNO(write(sk_bound, buf, 0), ENOTCONN);
}
END_TEST()

FN_TEST(recv)
{
	char buf[1] = { 'z' };

	TEST_ERRNO(recv(sk_unbound, buf, 1, 0), EAGAIN);
	TEST_ERRNO(recv(sk_unbound, buf, 0, 0), EAGAIN);
	TEST_ERRNO(read(sk_unbound, buf, 1), EAGAIN);
	TEST_SUCC(read(sk_unbound, buf, 0));

	TEST_ERRNO(recv(sk_bound, buf, 1, 0), EAGAIN);
	TEST_ERRNO(recv(sk_bound, buf, 0, 0), EAGAIN);
	TEST_ERRNO(read(sk_bound, buf, 1), EAGAIN);
	TEST_SUCC(read(sk_bound, buf, 0));
}
END_TEST()

FN_TEST(blocking_recv)
{
	int sk1, sk2;
	int pid;
	char buf[20];

	// Setup

	sk1 = TEST_SUCC(socket(PF_UNIX, SOCK_DGRAM, 0));
	TEST_SUCC(bind(sk1, (struct sockaddr *)&UNIX_ADDR("\0"),
		       PATH_OFFSET + 1));

	sk2 = TEST_SUCC(socket(PF_UNIX, SOCK_DGRAM | SOCK_NONBLOCK, 0));
	TEST_SUCC(connect(sk2, (struct sockaddr *)&UNIX_ADDR("\0"),
			  PATH_OFFSET + 1));

#define MAKE_TEST(child, retval)                                  \
	pid = TEST_SUCC(fork());                                  \
	if (pid == 0) {                                           \
		usleep(300 * 1000);                               \
		CHECK(child);                                     \
		exit(0);                                          \
	}                                                         \
                                                                  \
	TEST_RES(recv(sk1, buf, sizeof(buf), 0), _ret == retval); \
	TEST_SUCC(wait(NULL));

	// Test 1: Sends a message resumes the blocked receiving
	MAKE_TEST(send(sk2, "hello", 5, 0), 5);

	// Test 2: Shuts down for reading resumes the blocked receiving
	MAKE_TEST(shutdown(sk1, SHUT_RD), 0);

#undef MAKE_TEST

	// Clean up

	TEST_SUCC(close(sk1));
	TEST_SUCC(close(sk2));
}
END_TEST()

FN_TEST(send_recv_trunc)
{
	char buf[1];

	TEST_SUCC(send(sk_connected, "abc", 3, 0));
	TEST_SUCC(send(sk_connected, "def", 3, 0));
	TEST_SUCC(send(sk_connected, "hij", 3, 0));

	TEST_RES(recv(sk_bound, buf, 1, 0), _ret == 1 && buf[0] == 'a');
	TEST_RES(recv(sk_bound, buf, 0, 0), _ret == 0);
	TEST_RES(recv(sk_bound, buf, 1, 0), _ret == 1 && buf[0] == 'h');
}
END_TEST()

FN_TEST(send_recv_zero)
{
	char buf[1];

	buf[0] = 'a';
	TEST_SUCC(send(sk_connected, buf, 1, 0));
	buf[0] = 'b';
	TEST_SUCC(send(sk_connected, buf, 0, 0));
	buf[0] = 'c';
	TEST_SUCC(send(sk_connected, buf, 0, 0));
	buf[0] = 'd';
	TEST_SUCC(send(sk_connected, buf, 1, 0));

	TEST_RES(recv(sk_bound, buf, 1, 0), _ret == 1 && buf[0] == 'a');
	TEST_RES(recv(sk_bound, buf, 1, 0), _ret == 0 && buf[0] == 'a');
	TEST_RES(recv(sk_bound, buf, 1, 0), _ret == 0 && buf[0] == 'a');
	TEST_RES(recv(sk_bound, buf, 1, 0), _ret == 1 && buf[0] == 'd');
}
END_TEST()

FN_TEST(shutdown_connected)
{
	int fildes[2];

	TEST_SUCC(socketpair(PF_UNIX, SOCK_DGRAM, 0, fildes));

	TEST_SUCC(shutdown(fildes[0], SHUT_RD));
	TEST_SUCC(shutdown(fildes[0], SHUT_WR));
	TEST_SUCC(shutdown(fildes[0], SHUT_RDWR));

	TEST_SUCC(shutdown(fildes[0], SHUT_RD));
	TEST_SUCC(shutdown(fildes[0], SHUT_WR));
	TEST_SUCC(shutdown(fildes[0], SHUT_RDWR));

	TEST_SUCC(close(fildes[0]));
	TEST_SUCC(close(fildes[1]));
}
END_TEST()

FN_TEST(shutdown_close_send)
{
	int fildes[2];
	struct sockaddr_un addr;
	socklen_t addrlen;

	TEST_SUCC(socketpair(PF_UNIX, SOCK_DGRAM, 0, fildes));
	TEST_SUCC(bind(fildes[0], (struct sockaddr *)&UNIX_ADDR("\0X"),
		       PATH_OFFSET + 2));

	// Test 1: Sending a message after shutting down the receiver.
	TEST_SUCC(shutdown(fildes[0], SHUT_RDWR));
	TEST_ERRNO(send(fildes[1], "", 0, 0), EPIPE);

	// The socket is still connected.
	addrlen = sizeof(addr);
	TEST_RES(getpeername(fildes[1], (struct sockaddr *)&addr, &addrlen),
		 addrlen == PATH_OFFSET + 2 && memcmp(&addr, &UNIX_ADDR("\0X"),
						      PATH_OFFSET + 2) == 0);

	// Test 2: Sending a message after closing the receiver.
	TEST_SUCC(close(fildes[0]));
	TEST_ERRNO(send(fildes[1], "", 0, 0), ECONNREFUSED);

	// The socket is no longer connected.
	TEST_ERRNO(send(fildes[1], "", 0, 0), ENOTCONN);
	TEST_ERRNO(getpeername(fildes[1], (struct sockaddr *)&addr, &addrlen),
		   ENOTCONN);

	TEST_SUCC(close(fildes[1]));
}
END_TEST()

FN_TEST(poll)
{
	int sk;
	struct pollfd pfd = { .events = POLLIN | POLLOUT | POLLRDHUP };

	sk = TEST_SUCC(socket(PF_UNIX, SOCK_DGRAM, 0));
	pfd.fd = sk;

	TEST_RES(poll(&pfd, 1, 0), pfd.revents == POLLOUT);

	TEST_SUCC(shutdown(sk, SHUT_WR));
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == POLLOUT);

	TEST_SUCC(shutdown(sk, SHUT_RD));
	TEST_RES(poll(&pfd, 1, 0),
		 pfd.revents == (POLLIN | POLLOUT | POLLRDHUP | POLLHUP));

	TEST_SUCC(
		bind(sk, (struct sockaddr *)&UNIX_ADDR("\0"), PATH_OFFSET + 1));
	TEST_RES(poll(&pfd, 1, 0),
		 pfd.revents == (POLLIN | POLLOUT | POLLRDHUP | POLLHUP));

	TEST_SUCC(connect(sk, (struct sockaddr *)&BOUND_ADDR, BOUND_ADDRLEN));
	TEST_RES(poll(&pfd, 1, 0),
		 pfd.revents == (POLLIN | POLLOUT | POLLRDHUP | POLLHUP));

	TEST_SUCC(close(sk));
}
END_TEST()

FN_TEST(poll_connected_close)
{
	int fildes[2];
	struct pollfd pfd = { .events = POLLIN | POLLOUT | POLLRDHUP };

	TEST_SUCC(socketpair(PF_UNIX, SOCK_DGRAM, 0, fildes));

	pfd.fd = fildes[1];
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == POLLOUT);

	TEST_SUCC(close(fildes[0]));

	pfd.fd = fildes[1];
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == POLLOUT);

	TEST_SUCC(close(fildes[1]));
}
END_TEST()

FN_TEST(poll_connected_shutdown)
{
	int fildes[2];
	struct pollfd pfd = { .events = POLLIN | POLLOUT | POLLRDHUP };

#define MAKE_TEST(shut, ev1)                                   \
	TEST_SUCC(socketpair(PF_UNIX, SOCK_DGRAM, 0, fildes)); \
                                                               \
	TEST_SUCC(shutdown(fildes[0], shut));                  \
                                                               \
	pfd.fd = fildes[0];                                    \
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (ev1));      \
                                                               \
	pfd.fd = fildes[1];                                    \
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == POLLOUT);    \
                                                               \
	TEST_SUCC(close(fildes[0]));                           \
	TEST_SUCC(close(fildes[1]));

	MAKE_TEST(SHUT_RD, POLLIN | POLLOUT | POLLRDHUP);

	MAKE_TEST(SHUT_WR, POLLOUT);

	MAKE_TEST(SHUT_RDWR, POLLIN | POLLOUT | POLLRDHUP | POLLHUP);

#undef MAKE_TEST
}
END_TEST()

// See also `zero_reads_always_succeed` in `pipe_err.c`
FN_TEST(zero_recvs_may_fail)
{
	int fildes[2];
	char buf[1] = { 'z' };

	TEST_SUCC(socketpair(AF_UNIX, SOCK_DGRAM | SOCK_NONBLOCK, 0, fildes));

	TEST_ERRNO(recv(fildes[0], buf, 0, 0), EAGAIN);

	TEST_RES(send(fildes[1], buf, 1, 0), _ret == 1);
	TEST_SUCC(recv(fildes[0], buf, 0, 0));

	TEST_SUCC(close(fildes[0]));
	TEST_SUCC(close(fildes[1]));
}
END_TEST()

// See also `zero_writes_always_succeed` in `pipe_err.c`
FN_TEST(zero_sends_may_fail)
{
	int fildes[2];
	char buf[1] = { 'z' };

	TEST_SUCC(socketpair(AF_UNIX, SOCK_DGRAM | SOCK_NONBLOCK, 0, fildes));

	TEST_SUCC(send(fildes[1], buf, 0, 0));

	TEST_SUCC(close(fildes[0]));
	TEST_ERRNO(send(fildes[1], buf, 0, 0), ECONNREFUSED);

	TEST_SUCC(close(fildes[1]));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(sk_unbound));

	CHECK(close(sk_bound));

	CHECK(close(sk_connected));

	CHECK(unlink(BOUND_ADDR.sun_path));
}
END_SETUP()
