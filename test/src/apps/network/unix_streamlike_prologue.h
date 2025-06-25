/* SPDX-License-Identifier: MPL-2.0 */

#define _GNU_SOURCE

#include <sys/socket.h>
#include <sys/un.h>
#include <sys/poll.h>
#include <sys/epoll.h>
#include <sys/wait.h>
#include <fcntl.h>
#include <unistd.h>
#include <stddef.h>

#include "../test.h"

FN_SETUP(general)
{
	signal(SIGPIPE, SIG_IGN);
}
END_SETUP()

#define PATH_OFFSET offsetof(struct sockaddr_un, sun_path)

FN_TEST(socket_addresses)
{
	int sk;
	socklen_t addrlen;
	struct sockaddr_un addr;

#define MIN(a, b) ((a) < (b) ? (a) : (b))

#define MAKE_TEST(path, path_copy_len, path_len_to_kernel, path_buf_len,       \
		  path_len_from_kernel, path_from_kernel)                      \
	sk = TEST_SUCC(socket(PF_UNIX, SOCK_TYPE, 0));                         \
                                                                               \
	memset(&addr, 0, sizeof(addr));                                        \
	addr.sun_family = AF_UNIX;                                             \
	memcpy(addr.sun_path, path, path_copy_len);                            \
                                                                               \
	TEST_SUCC(bind(sk, (struct sockaddr *)&addr,                           \
		       PATH_OFFSET + path_len_to_kernel));                     \
                                                                               \
	memset(&addr, 0, sizeof(addr));                                        \
                                                                               \
	addrlen = path_buf_len + PATH_OFFSET;                                  \
	TEST_RES(                                                              \
		getsockname(sk, (struct sockaddr *)&addr, &addrlen),           \
		addrlen == PATH_OFFSET + path_len_from_kernel &&               \
			0 == memcmp(addr.sun_path, path_from_kernel,           \
				    MIN(path_buf_len, path_len_from_kernel))); \
                                                                               \
	TEST_SUCC(close(sk));

#define LONG_PATH \
	"/tmp/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
	_Static_assert(sizeof(LONG_PATH) == sizeof(addr.sun_path),
		       "LONG_PATH has a wrong length");

	MAKE_TEST("/tmp/R0", 8, 8, 8, 8, "/tmp/R0");
	TEST_SUCC(unlink("/tmp/R0"));

	MAKE_TEST("/tmp/R1", 8, 9, 8, 8, "/tmp/R1");
	TEST_SUCC(unlink("/tmp/R1"));

	MAKE_TEST("/tmp/R2", 6, 6, 8, 7, "/tmp/R");
	TEST_SUCC(unlink("/tmp/R"));

	MAKE_TEST("/tmp/R3", 7, 7, 8, 8, "/tmp/R3");
	TEST_SUCC(unlink("/tmp/R3"));

	MAKE_TEST("/tmp/R4", 7, 7, 7, 8, "/tmp/R4");
	TEST_SUCC(unlink("/tmp/R4"));

	MAKE_TEST("/tmp/R5", 7, 7, 6, 8, "/tmp/R");
	TEST_SUCC(unlink("/tmp/R5"));

	MAKE_TEST("/tmp/R6", 7, 7, 0, 8, "");
	TEST_SUCC(unlink("/tmp/R6"));

	MAKE_TEST(LONG_PATH, 107, 107, 108, 108, LONG_PATH);
	TEST_SUCC(unlink(LONG_PATH));

	MAKE_TEST(LONG_PATH "a", 108, 108, 108, 109, LONG_PATH "a");
	TEST_SUCC(unlink(LONG_PATH "a"));

#undef LONG_PATH
#undef MAKE_TEST

	sk = TEST_SUCC(socket(PF_UNIX, SOCK_TYPE, 0));

	TEST_ERRNO(bind(sk, (struct sockaddr *)&addr, -1), EINVAL);
	TEST_ERRNO(bind(sk, (struct sockaddr *)&addr, PATH_OFFSET - 1), EINVAL);
	TEST_ERRNO(bind(sk, (struct sockaddr *)&addr, sizeof(addr) + 1),
		   EINVAL);

	TEST_SUCC(close(sk));
}
END_TEST()

static int sk_unbound;
static int sk_bound;
static int sk_listen;
static int sk_connected;
static int sk_accepted;

#define UNIX_ADDR(path) \
	((struct sockaddr_un){ .sun_family = AF_UNIX, .sun_path = path })

#define UNNAMED_ADDR UNIX_ADDR("")
#define UNNAMED_ADDRLEN PATH_OFFSET

#define BOUND_ADDR UNIX_ADDR("//tmp/B0")
#define BOUND_ADDRLEN (PATH_OFFSET + 9)

#define LISTEN_ADDR UNIX_ADDR("/tmp//L0")
#define LISTEN_ADDRLEN (PATH_OFFSET + 9)

#define LISTEN_ADDR2 UNIX_ADDR("/tmp/L0")
#define LISTEN_ADDRLEN2 (PATH_OFFSET + 8)

FN_SETUP(unbound)
{
	sk_unbound = CHECK(socket(PF_UNIX, SOCK_TYPE | SOCK_NONBLOCK, 0));
}
END_SETUP()

FN_SETUP(bound)
{
	sk_bound = CHECK(socket(PF_UNIX, SOCK_TYPE | SOCK_NONBLOCK, 0));

	CHECK(bind(sk_bound, (struct sockaddr *)&BOUND_ADDR, BOUND_ADDRLEN));
}
END_SETUP()

FN_SETUP(listen)
{
	sk_listen = CHECK(socket(PF_UNIX, SOCK_TYPE | SOCK_NONBLOCK, 0));

	CHECK(bind(sk_listen, (struct sockaddr *)&LISTEN_ADDR, LISTEN_ADDRLEN));

	CHECK(listen(sk_listen, 1));
}
END_SETUP()

FN_SETUP(connected)
{
	sk_connected = CHECK(socket(PF_UNIX, SOCK_TYPE | SOCK_NONBLOCK, 0));

	CHECK(connect(sk_connected, (struct sockaddr *)&LISTEN_ADDR2,
		      LISTEN_ADDRLEN2));
}
END_SETUP()

FN_SETUP(accepted)
{
	sk_accepted = CHECK(accept(sk_listen, NULL, NULL));
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
	TEST_RES(getsockname(sk_listen, (struct sockaddr *)&addr, &addrlen),
		 addrlen == LISTEN_ADDRLEN &&
			 memcmp(&addr, &LISTEN_ADDR, LISTEN_ADDRLEN) == 0);

	addrlen = sizeof(addr);
	TEST_RES(getsockname(sk_connected, (struct sockaddr *)&addr, &addrlen),
		 addrlen == UNNAMED_ADDRLEN &&
			 memcmp(&addr, &UNNAMED_ADDR, UNNAMED_ADDRLEN) == 0);

	addrlen = sizeof(addr);
	TEST_RES(getsockname(sk_accepted, (struct sockaddr *)&addr, &addrlen),
		 addrlen == LISTEN_ADDRLEN &&
			 memcmp(&addr, &LISTEN_ADDR, LISTEN_ADDRLEN) == 0);
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
	TEST_ERRNO(getpeername(sk_listen, (struct sockaddr *)&addr, &addrlen),
		   ENOTCONN);

	addrlen = sizeof(addr);
	TEST_RES(getpeername(sk_connected, (struct sockaddr *)&addr, &addrlen),
		 addrlen == LISTEN_ADDRLEN &&
			 memcmp(&addr, &LISTEN_ADDR, LISTEN_ADDRLEN) == 0);

	addrlen = sizeof(addr);
	TEST_RES(getpeername(sk_accepted, (struct sockaddr *)&addr, &addrlen),
		 addrlen == UNNAMED_ADDRLEN &&
			 memcmp(&addr, &UNNAMED_ADDR, UNNAMED_ADDRLEN) == 0);
}
END_TEST()

FN_TEST(bind)
{
	TEST_ERRNO(bind(sk_bound, (struct sockaddr *)&UNIX_ADDR("\0Z"),
			PATH_OFFSET + 1),
		   EINVAL);

	TEST_ERRNO(bind(sk_listen, (struct sockaddr *)&UNIX_ADDR("\0Z"),
			PATH_OFFSET + 1),
		   EINVAL);

	TEST_SUCC(bind(sk_bound, (struct sockaddr *)&UNNAMED_ADDR,
		       UNNAMED_ADDRLEN));

	TEST_SUCC(bind(sk_listen, (struct sockaddr *)&UNNAMED_ADDR,
		       UNNAMED_ADDRLEN));
}
END_TEST()

FN_TEST(bind_connected)
{
	int fildes[2];
	struct sockaddr_un addr;
	socklen_t addrlen;

	TEST_SUCC(socketpair(PF_UNIX, SOCK_TYPE, 0, fildes));

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

	TEST_SUCC(close(fildes[0]));
	TEST_SUCC(close(fildes[1]));
}
END_TEST()

FN_TEST(connect)
{
	TEST_ERRNO(connect(sk_unbound, (struct sockaddr *)&BOUND_ADDR,
			   BOUND_ADDRLEN),
		   ECONNREFUSED);

	TEST_ERRNO(connect(sk_bound, (struct sockaddr *)&BOUND_ADDR,
			   BOUND_ADDRLEN),
		   ECONNREFUSED);

	TEST_ERRNO(connect(sk_listen, (struct sockaddr *)&LISTEN_ADDR,
			   LISTEN_ADDRLEN),
		   EINVAL);

	TEST_ERRNO(connect(sk_connected, (struct sockaddr *)&LISTEN_ADDR,
			   LISTEN_ADDRLEN),
		   EISCONN);

	TEST_ERRNO(connect(sk_connected, (struct sockaddr *)&LISTEN_ADDR,
			   LISTEN_ADDRLEN),
		   EISCONN);
}
END_TEST()

FN_TEST(listen)
{
	TEST_ERRNO(listen(sk_unbound, 10), EINVAL);

	TEST_SUCC(listen(sk_listen, 10));

	TEST_ERRNO(listen(sk_connected, 10), EINVAL);

	TEST_ERRNO(listen(sk_accepted, 10), EINVAL);
}
END_TEST()

FN_TEST(accept)
{
	TEST_ERRNO(accept(sk_unbound, NULL, NULL), EINVAL);

	TEST_ERRNO(accept(sk_bound, NULL, NULL), EINVAL);

	TEST_ERRNO(accept(sk_connected, NULL, NULL), EINVAL);

	TEST_ERRNO(accept(sk_accepted, NULL, NULL), EINVAL);
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

	TEST_ERRNO(send(sk_listen, buf, 1, 0), ENOTCONN);
	TEST_ERRNO(send(sk_listen, buf, 0, 0), ENOTCONN);
	TEST_ERRNO(write(sk_listen, buf, 1), ENOTCONN);
	TEST_ERRNO(write(sk_listen, buf, 0), ENOTCONN);
}
END_TEST()

FN_TEST(recv)
{
	char buf[1] = { 'z' };

	TEST_ERRNO(recv(sk_unbound, buf, 1, 0), EINVAL);
	TEST_ERRNO(recv(sk_unbound, buf, 0, 0), EINVAL);
	TEST_ERRNO(read(sk_unbound, buf, 1), EINVAL);
	TEST_SUCC(read(sk_unbound, buf, 0));

	TEST_ERRNO(recv(sk_bound, buf, 1, 0), EINVAL);
	TEST_ERRNO(recv(sk_bound, buf, 0, 0), EINVAL);
	TEST_ERRNO(read(sk_bound, buf, 1), EINVAL);
	TEST_SUCC(read(sk_bound, buf, 0));

	TEST_ERRNO(recv(sk_listen, buf, 1, 0), EINVAL);
	TEST_ERRNO(recv(sk_listen, buf, 0, 0), EINVAL);
	TEST_ERRNO(read(sk_listen, buf, 1), EINVAL);
	TEST_SUCC(read(sk_listen, buf, 0));

	TEST_ERRNO(recv(sk_connected, buf, 1, 0), EAGAIN);
	TEST_ERRNO(recv(sk_connected, buf, 0, 0), EAGAIN);
	TEST_ERRNO(read(sk_connected, buf, 1), EAGAIN);
	TEST_SUCC(read(sk_connected, buf, 0));
}
END_TEST()

FN_TEST(blocking_connect)
{
	int i;
	int sk, sks[4];
	int pid;

	// Setup

	sk = TEST_SUCC(socket(PF_UNIX, SOCK_TYPE, 0));
	TEST_SUCC(
		bind(sk, (struct sockaddr *)&UNIX_ADDR("\0"), PATH_OFFSET + 1));
	TEST_SUCC(listen(sk, 2));

	for (i = 0; i < 3; ++i) {
		sks[i] = TEST_SUCC(
			socket(PF_UNIX, SOCK_TYPE | SOCK_NONBLOCK, 0));
		TEST_SUCC(connect(sks[i], (struct sockaddr *)&UNIX_ADDR("\0"),
				  PATH_OFFSET + 1));
	}

#define MAKE_TEST(child, parent, errno)                                    \
	sks[i] = TEST_SUCC(socket(PF_UNIX, SOCK_TYPE | SOCK_NONBLOCK, 0)); \
	TEST_ERRNO(connect(sks[i], (struct sockaddr *)&UNIX_ADDR("\0"),    \
			   PATH_OFFSET + 1),                               \
		   EAGAIN);                                                \
	TEST_SUCC(close(sks[i]));                                          \
                                                                           \
	pid = TEST_SUCC(fork());                                           \
	if (pid == 0) {                                                    \
		usleep(300 * 1000);                                        \
		CHECK(child);                                              \
		exit(0);                                                   \
	}                                                                  \
	TEST_SUCC(parent);                                                 \
                                                                           \
	sks[i] = TEST_SUCC(socket(PF_UNIX, SOCK_TYPE, 0));                 \
	TEST_ERRNO(connect(sks[i], (struct sockaddr *)&UNIX_ADDR("\0"),    \
			   PATH_OFFSET + 1),                               \
		   errno);                                                 \
                                                                           \
	TEST_SUCC(close(sks[i]));                                          \
	TEST_SUCC(wait(NULL));

	// Test 1: Accepting a connection resumes the blocked connection request
	MAKE_TEST(accept(sk, NULL, NULL), 0, 0);

	// Test 2: Resetting the backlog resumes the blocked connection request
	MAKE_TEST(listen(sk, 3), 0, 0);

	// Test 3: Closing the listener resumes the blocked connection request
	MAKE_TEST(close(sk), close(sk), ECONNREFUSED);

#undef MAKE_TEST

	// Clean up

	for (i = 0; i < 3; ++i)
		TEST_SUCC(close(sks[i]));
}
END_TEST()

FN_TEST(ns_path)
{
	int fd;

	fd = TEST_SUCC(creat("/tmp/.good", 0644));
	TEST_ERRNO(bind(sk_unbound, (struct sockaddr *)&UNIX_ADDR("/tmp/.good"),
			sizeof(struct sockaddr)),
		   EADDRINUSE);
	TEST_ERRNO(connect(sk_unbound,
			   (struct sockaddr *)&UNIX_ADDR("/tmp/.good"),
			   sizeof(struct sockaddr)),
		   ECONNREFUSED);
	TEST_SUCC(close(fd));
	TEST_SUCC(unlink("/tmp/.good"));

	fd = TEST_SUCC(creat("/tmp/.bad", 0000));
	TEST_ERRNO(bind(sk_unbound, (struct sockaddr *)&UNIX_ADDR("/tmp/.bad"),
			sizeof(struct sockaddr)),
		   EADDRINUSE);
	TEST_ERRNO(connect(sk_unbound,
			   (struct sockaddr *)&UNIX_ADDR("/tmp/.bad"),
			   sizeof(struct sockaddr)),
		   EACCES);
	TEST_SUCC(close(fd));
	TEST_SUCC(unlink("/tmp/.bad"));
}
END_TEST()

FN_TEST(ns_abs)
{
	int sk, sk2;
	struct sockaddr_un addr;
	socklen_t addrlen;

	sk = TEST_SUCC(socket(PF_UNIX, SOCK_TYPE, 0));

	TEST_SUCC(bind(sk, (struct sockaddr *)&UNIX_ADDR(""), PATH_OFFSET));
	addrlen = sizeof(addr);
	TEST_RES(getsockname(sk, (struct sockaddr *)&addr, &addrlen),
		 addrlen == PATH_OFFSET + 6 && addr.sun_path[0] == '\0');

	sk2 = TEST_SUCC(socket(PF_UNIX, SOCK_TYPE, 0));

	TEST_ERRNO(bind(sk2, (struct sockaddr *)&addr, addrlen), EADDRINUSE);
	TEST_ERRNO(connect(sk2, (struct sockaddr *)&addr, addrlen),
		   ECONNREFUSED);
	TEST_SUCC(listen(sk, 1));
	TEST_SUCC(connect(sk2, (struct sockaddr *)&addr, addrlen));

	TEST_SUCC(close(sk));
	TEST_SUCC(close(sk2));

	sk = TEST_SUCC(socket(PF_UNIX, SOCK_TYPE, 0));
	TEST_ERRNO(connect(sk, (struct sockaddr *)&addr, addrlen),
		   ECONNREFUSED);
	TEST_SUCC(bind(sk, (struct sockaddr *)&addr, addrlen));
	TEST_SUCC(close(sk));
}
END_TEST()

FN_TEST(shutdown_connected)
{
	int fildes[2];

	TEST_SUCC(socketpair(PF_UNIX, SOCK_TYPE, 0, fildes));

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

FN_TEST(poll_unbound)
{
	int sk;
	struct pollfd pfd = { .events = POLLIN | POLLOUT | POLLRDHUP };

	sk = TEST_SUCC(socket(PF_UNIX, SOCK_TYPE, 0));
	pfd.fd = sk;

	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLOUT | POLLHUP));

	TEST_SUCC(shutdown(sk, SHUT_WR));
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLOUT | POLLHUP));

	TEST_SUCC(shutdown(sk, SHUT_RD));
	TEST_RES(poll(&pfd, 1, 0),
		 pfd.revents == (POLLIN | POLLOUT | POLLRDHUP | POLLHUP));

	TEST_SUCC(
		bind(sk, (struct sockaddr *)&UNIX_ADDR("\0"), PATH_OFFSET + 1));
	TEST_SUCC(listen(sk, 10));

	TEST_RES(poll(&pfd, 1, 0),
		 pfd.revents == (POLLIN | POLLRDHUP | POLLHUP));

	TEST_SUCC(close(sk));
}
END_TEST()

FN_TEST(poll_listen)
{
	int sk;
	struct pollfd pfd = { .events = POLLIN | POLLOUT | POLLRDHUP };

	sk = TEST_SUCC(socket(PF_UNIX, SOCK_TYPE, 0));
	pfd.fd = sk;

	TEST_SUCC(
		bind(sk, (struct sockaddr *)&UNIX_ADDR("\0"), PATH_OFFSET + 1));
	TEST_SUCC(listen(sk, 10));

	TEST_RES(poll(&pfd, 1, 0), pfd.revents == 0);

	TEST_SUCC(shutdown(sk, SHUT_RD));
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLIN | POLLRDHUP));

	TEST_SUCC(shutdown(sk, SHUT_WR));
	TEST_RES(poll(&pfd, 1, 0),
		 pfd.revents == (POLLIN | POLLRDHUP | POLLHUP));

	TEST_SUCC(close(sk));
}
END_TEST()

FN_TEST(poll_connected_close)
{
	int fildes[2];
	struct pollfd pfd = { .events = POLLIN | POLLOUT | POLLRDHUP };

	TEST_SUCC(socketpair(PF_UNIX, SOCK_TYPE, 0, fildes));

	pfd.fd = fildes[1];
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == POLLOUT);

	TEST_SUCC(close(fildes[0]));

	pfd.fd = fildes[1];
	TEST_RES(poll(&pfd, 1, 0),
		 pfd.revents == (POLLIN | POLLOUT | POLLRDHUP | POLLHUP));

	TEST_SUCC(close(fildes[1]));
}
END_TEST()

FN_TEST(poll_connected_shutdown)
{
	int fildes[2];
	struct pollfd pfd = { .events = POLLIN | POLLOUT | POLLRDHUP };

#define MAKE_TEST(shut, ev1, ev2)                             \
	TEST_SUCC(socketpair(PF_UNIX, SOCK_TYPE, 0, fildes)); \
                                                              \
	TEST_SUCC(shutdown(fildes[0], shut));                 \
                                                              \
	pfd.fd = fildes[0];                                   \
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (ev1));     \
                                                              \
	pfd.fd = fildes[1];                                   \
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (ev2));     \
                                                              \
	TEST_SUCC(close(fildes[0]));                          \
	TEST_SUCC(close(fildes[1]));

	MAKE_TEST(SHUT_RD, POLLIN | POLLOUT | POLLRDHUP, POLLOUT);

	MAKE_TEST(SHUT_WR, POLLOUT, POLLIN | POLLOUT | POLLRDHUP);

	MAKE_TEST(SHUT_RDWR, POLLIN | POLLOUT | POLLRDHUP | POLLHUP,
		  POLLIN | POLLOUT | POLLRDHUP | POLLHUP);

#undef MAKE_TEST
}
END_TEST()

FN_TEST(epoll)
{
	int sk2_listen, sk2_connected, sk2_accepted;
	int epfd_listen, epfd_connected;
	struct epoll_event ev;

	// Setup

	sk2_listen = TEST_SUCC(socket(PF_UNIX, SOCK_TYPE, 0));
	sk2_connected = TEST_SUCC(socket(PF_UNIX, SOCK_TYPE, 0));

	epfd_listen = TEST_SUCC(epoll_create1(0));
	ev.events = EPOLLIN;
	ev.data.fd = sk2_listen;
	TEST_SUCC(epoll_ctl(epfd_listen, EPOLL_CTL_ADD, sk2_listen, &ev));

	epfd_connected = TEST_SUCC(epoll_create1(0));
	ev.events = EPOLLIN;
	ev.data.fd = sk2_connected;
	TEST_SUCC(epoll_ctl(epfd_connected, EPOLL_CTL_ADD, sk2_connected, &ev));

	// Test 1: Switch from the unbound state to the listening state

	TEST_SUCC(bind(sk2_listen, (struct sockaddr *)&UNIX_ADDR("\0"),
		       PATH_OFFSET + 1));
	TEST_SUCC(listen(sk2_listen, 10));
	TEST_RES(epoll_wait(epfd_listen, &ev, 1, 0), _ret == 0);

	TEST_SUCC(connect(sk2_connected, (struct sockaddr *)&UNIX_ADDR("\0"),
			  PATH_OFFSET + 1));
	ev.data.fd = -1;
	TEST_RES(epoll_wait(epfd_listen, &ev, 1, 0),
		 _ret == 1 && ev.data.fd == sk2_listen);

	// Test 2: Switch from the unbound state to the connected state

	TEST_RES(epoll_wait(epfd_connected, &ev, 1, 0), _ret == 0);

	sk2_accepted = TEST_SUCC(accept(sk2_listen, NULL, 0));
	TEST_SUCC(write(sk2_accepted, "", 1));

	ev.data.fd = -1;
	TEST_RES(epoll_wait(epfd_connected, &ev, 1, 0),
		 _ret == 1 && ev.data.fd == sk2_connected);

	// Clean up

	TEST_SUCC(close(epfd_listen));
	TEST_SUCC(close(epfd_connected));
	TEST_SUCC(close(sk2_connected));
	TEST_SUCC(close(sk2_accepted));
	TEST_SUCC(close(sk2_listen));
}
END_TEST()

// See also `zero_reads_always_succeed` in `pipe_err.c`
FN_TEST(zero_recvs_may_fail)
{
	int fildes[2];
	char buf[1] = { 'z' };

	TEST_SUCC(socketpair(AF_UNIX, SOCK_TYPE | SOCK_NONBLOCK, 0, fildes));

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

	TEST_SUCC(socketpair(AF_UNIX, SOCK_TYPE | SOCK_NONBLOCK, 0, fildes));

	TEST_SUCC(send(fildes[1], buf, 0, 0));

	TEST_SUCC(close(fildes[0]));
	TEST_ERRNO(send(fildes[1], buf, 0, 0), EPIPE);

	TEST_SUCC(close(fildes[1]));
}
END_TEST()
