// SPDX-License-Identifier: MPL-2.0

#include <unistd.h>
#include <sys/signal.h>
#include <sys/socket.h>
#include <sys/poll.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <fcntl.h>

#include "../test.h"

static struct sockaddr_in sk_addr;

#define C_PORT htons(0x1234)
#define S_PORT htons(0x1235)

FN_SETUP(general)
{
	sk_addr.sin_family = AF_INET;
	sk_addr.sin_port = htons(8080);
	CHECK(inet_aton("127.0.0.1", &sk_addr.sin_addr));

	signal(SIGPIPE, SIG_IGN);
}
END_SETUP()

static int sk_unbound;
static int sk_bound;
static int sk_listen;
static int sk_connected;
static int sk_accepted;

FN_SETUP(unbound)
{
	sk_unbound = CHECK(socket(PF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0));
}
END_SETUP()

FN_SETUP(bound)
{
	sk_bound = CHECK(socket(PF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0));

	sk_addr.sin_port = C_PORT;
	CHECK(bind(sk_bound, (struct sockaddr *)&sk_addr, sizeof(sk_addr)));
}
END_SETUP()

FN_SETUP(listen)
{
	sk_listen = CHECK(socket(PF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0));

	sk_addr.sin_port = S_PORT;
	CHECK(bind(sk_listen, (struct sockaddr *)&sk_addr, sizeof(sk_addr)));

	CHECK(listen(sk_listen, 2));
}
END_SETUP()

FN_SETUP(connected)
{
	sk_connected = CHECK(socket(PF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0));

	sk_addr.sin_port = S_PORT;
	CHECK_WITH(connect(sk_connected, (struct sockaddr *)&sk_addr,
			   sizeof(sk_addr)),
		   _ret < 0 && errno == EINPROGRESS);
}
END_SETUP()

FN_SETUP(accpected)
{
	struct sockaddr addr;
	socklen_t addrlen = sizeof(addr);
	struct pollfd pfd = { .fd = sk_listen, .events = POLLIN };

	CHECK_WITH(poll(&pfd, 1, 1000),
		   _ret >= 0 && ((pfd.revents & (POLLIN | POLLOUT)) & POLLIN));

	sk_accepted = CHECK(accept(sk_listen, &addr, &addrlen));
}
END_SETUP()

FN_TEST(getsockname)
{
	struct sockaddr_in saddr = { .sin_port = 0xbeef };
	struct sockaddr *psaddr = (struct sockaddr *)&saddr;
	socklen_t addrlen = 0;

	TEST_RES(getsockname(sk_unbound, psaddr, &addrlen),
		 addrlen == sizeof(saddr) && saddr.sin_port == 0xbeef);

	TEST_RES(getsockname(sk_unbound, psaddr, &addrlen),
		 addrlen == sizeof(saddr) && saddr.sin_port == 0);

	TEST_RES(getsockname(sk_bound, psaddr, &addrlen),
		 addrlen == sizeof(saddr) && saddr.sin_port == C_PORT);

	TEST_RES(getsockname(sk_listen, psaddr, &addrlen),
		 addrlen == sizeof(saddr) && saddr.sin_port == S_PORT);

	TEST_RES(getsockname(sk_connected, psaddr, &addrlen),
		 addrlen == sizeof(saddr) && saddr.sin_port != S_PORT);

	TEST_RES(getsockname(sk_accepted, psaddr, &addrlen),
		 addrlen == sizeof(saddr) && saddr.sin_port == S_PORT);
}
END_TEST()

FN_TEST(getpeername)
{
	struct sockaddr_in saddr = { .sin_port = 0xbeef };
	struct sockaddr *psaddr = (struct sockaddr *)&saddr;
	socklen_t addrlen = sizeof(saddr);

	TEST_ERRNO(getpeername(sk_unbound, psaddr, &addrlen), ENOTCONN);

	TEST_ERRNO(getpeername(sk_bound, psaddr, &addrlen), ENOTCONN);

	TEST_ERRNO(getpeername(sk_listen, psaddr, &addrlen), ENOTCONN);

	TEST_RES(getpeername(sk_connected, psaddr, &addrlen),
		 addrlen == sizeof(saddr) && saddr.sin_port == S_PORT);

	TEST_RES(getpeername(sk_accepted, psaddr, &addrlen),
		 addrlen == sizeof(saddr) && saddr.sin_port != S_PORT);
}
END_TEST()

FN_TEST(peername_is_peer_sockname)
{
	struct sockaddr_in saddr = { .sin_port = 0xbeef };
	struct sockaddr *psaddr = (struct sockaddr *)&saddr;
	socklen_t addrlen = sizeof(saddr);
	int em_port;

	TEST_RES(getsockname(sk_connected, psaddr, &addrlen),
		 addrlen == sizeof(saddr));
	em_port = saddr.sin_port;

	TEST_RES(getpeername(sk_accepted, psaddr, &addrlen),
		 addrlen == sizeof(saddr) && saddr.sin_port == em_port);
}
END_TEST()

FN_TEST(send)
{
	char buf[1] = { 'z' };

	TEST_ERRNO(send(sk_unbound, buf, 1, 0), EPIPE);
	TEST_ERRNO(send(sk_unbound, buf, 0, 0), EPIPE);
	TEST_ERRNO(write(sk_unbound, buf, 1), EPIPE);
	TEST_ERRNO(write(sk_unbound, buf, 0), EPIPE);

	TEST_ERRNO(send(sk_bound, buf, 1, 0), EPIPE);
	TEST_ERRNO(send(sk_bound, buf, 0, 0), EPIPE);
	TEST_ERRNO(write(sk_bound, buf, 1), EPIPE);
	TEST_ERRNO(write(sk_bound, buf, 0), EPIPE);

	TEST_ERRNO(send(sk_listen, buf, 1, 0), EPIPE);
	TEST_ERRNO(send(sk_listen, buf, 0, 0), EPIPE);
	TEST_ERRNO(write(sk_listen, buf, 1), EPIPE);
	TEST_ERRNO(write(sk_listen, buf, 0), EPIPE);
}
END_TEST()

FN_TEST(recv)
{
	char buf[1] = { 'z' };

	TEST_ERRNO(recv(sk_unbound, buf, 1, 0), ENOTCONN);
	TEST_ERRNO(recv(sk_unbound, buf, 0, 0), ENOTCONN);
	TEST_ERRNO(read(sk_unbound, buf, 1), ENOTCONN);
	TEST_SUCC(read(sk_unbound, buf, 0));

	TEST_ERRNO(recv(sk_bound, buf, 1, 0), ENOTCONN);
	TEST_ERRNO(recv(sk_bound, buf, 0, 0), ENOTCONN);
	TEST_ERRNO(read(sk_bound, buf, 1), ENOTCONN);
	TEST_SUCC(read(sk_bound, buf, 0));

	TEST_ERRNO(recv(sk_listen, buf, 1, 0), ENOTCONN);
	TEST_ERRNO(recv(sk_listen, buf, 0, 0), ENOTCONN);
	TEST_ERRNO(read(sk_listen, buf, 1), ENOTCONN);
	TEST_SUCC(read(sk_listen, buf, 0));

	TEST_ERRNO(recv(sk_connected, buf, 1, 0), EAGAIN);
	TEST_ERRNO(recv(sk_connected, buf, 0, 0), EAGAIN);
	TEST_ERRNO(read(sk_connected, buf, 1), EAGAIN);
	TEST_SUCC(read(sk_connected, buf, 0));
}
END_TEST()

FN_TEST(send_and_recv)
{
	char buf[1];

	buf[0] = 'a';
	TEST_RES(send(sk_connected, buf, 1, 0), _ret == 1);

	buf[0] = 'b';
	sk_addr.sin_port = 0xbeef;
	TEST_RES(sendto(sk_accepted, buf, 1, 0, (struct sockaddr *)&sk_addr,
			sizeof(sk_addr)),
		 _ret == 1);

	TEST_RES(recv(sk_accepted, buf, 1, 0), buf[0] == 'a');

	TEST_RES(recv(sk_connected, buf, 1, 0), buf[0] == 'b');

	TEST_ERRNO(recv(sk_connected, buf, 1, 0), EAGAIN);
}
END_TEST()

FN_TEST(bind)
{
	struct sockaddr *psaddr = (struct sockaddr *)&sk_addr;
	socklen_t addrlen = sizeof(sk_addr);

	TEST_ERRNO(bind(sk_unbound, psaddr, addrlen - 1), EINVAL);

	TEST_ERRNO(bind(sk_bound, psaddr, addrlen), EINVAL);

	TEST_ERRNO(bind(sk_listen, psaddr, addrlen), EINVAL);

	TEST_ERRNO(bind(sk_connected, psaddr, addrlen), EINVAL);

	TEST_ERRNO(bind(sk_accepted, psaddr, addrlen), EINVAL);
}
END_TEST()

FN_TEST(bind_reuseaddr)
{
	sk_addr.sin_port = htons(8081);
	struct sockaddr *psaddr = (struct sockaddr *)&sk_addr;
	socklen_t addrlen = sizeof(sk_addr);

	int disable = 0;
	int enable = 1;
	int sk1 = TEST_SUCC(socket(PF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0));
	int sk2 = TEST_SUCC(socket(PF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0));

	TEST_SUCC(bind(sk1, psaddr, addrlen));

	TEST_ERRNO(bind(sk2, psaddr, addrlen), EADDRINUSE);

	// FIXME: The test will fail in Asterinas since it doesn't check
	// if the previous socket was bound with `SO_REUSEADDR`
	//
	// TEST_SUCC(setsockopt(sk1, SOL_SOCKET, SO_REUSEADDR, &disable,
	// 		     sizeof(disable)));
	// TEST_SUCC(setsockopt(sk2, SOL_SOCKET, SO_REUSEADDR, &enable,
	// 		     sizeof(enable)));
	// TEST_ERRNO(bind(sk2, psaddr, addrlen), EADDRINUSE);

	TEST_SUCC(setsockopt(sk1, SOL_SOCKET, SO_REUSEADDR, &enable,
			     sizeof(enable)));
	TEST_SUCC(setsockopt(sk2, SOL_SOCKET, SO_REUSEADDR, &disable,
			     sizeof(disable)));
	TEST_ERRNO(bind(sk2, psaddr, addrlen), EADDRINUSE);

	TEST_SUCC(setsockopt(sk1, SOL_SOCKET, SO_REUSEADDR, &enable,
			     sizeof(enable)));
	TEST_SUCC(setsockopt(sk2, SOL_SOCKET, SO_REUSEADDR, &enable,
			     sizeof(enable)));
	TEST_SUCC(bind(sk2, psaddr, addrlen));

	TEST_SUCC(close(sk1));
	TEST_SUCC(close(sk2));
}
END_TEST()

FN_TEST(listen)
{
	// The second `listen` does nothing but succeed.
	// TODO: Will it update the backlog?
	TEST_SUCC(listen(sk_listen, 2));

	TEST_ERRNO(listen(sk_connected, 2), EINVAL);

	TEST_ERRNO(listen(sk_accepted, 2), EINVAL);
}
END_TEST()

FN_TEST(accept)
{
	struct sockaddr_in saddr;
	struct sockaddr *psaddr = (struct sockaddr *)&saddr;
	socklen_t addrlen = sizeof(saddr);

	TEST_ERRNO(accept(sk_unbound, psaddr, &addrlen), EINVAL);

	TEST_ERRNO(accept(sk_bound, psaddr, &addrlen), EINVAL);

	TEST_ERRNO(accept(sk_listen, psaddr, &addrlen), EAGAIN);

	TEST_ERRNO(accept(sk_connected, psaddr, &addrlen), EINVAL);

	TEST_ERRNO(accept(sk_accepted, psaddr, &addrlen), EINVAL);
}
END_TEST()

FN_TEST(poll)
{
	struct pollfd pfd = { .events = POLLIN | POLLOUT };

	pfd.fd = sk_unbound;
	TEST_RES(poll(&pfd, 1, 0),
		 (pfd.revents & (POLLIN | POLLOUT)) == POLLOUT);

	pfd.fd = sk_bound;
	TEST_RES(poll(&pfd, 1, 0),
		 (pfd.revents & (POLLIN | POLLOUT)) == POLLOUT);

	pfd.fd = sk_listen;
	TEST_RES(poll(&pfd, 1, 0), (pfd.revents & (POLLIN | POLLOUT)) == 0);

	pfd.fd = sk_connected;
	TEST_RES(poll(&pfd, 1, 0),
		 (pfd.revents & (POLLIN | POLLOUT)) == POLLOUT);

	pfd.fd = sk_accepted;
	TEST_RES(poll(&pfd, 1, 0),
		 (pfd.revents & (POLLIN | POLLOUT)) == POLLOUT);
}
END_TEST()

FN_TEST(connect)
{
	struct sockaddr *psaddr = (struct sockaddr *)&sk_addr;
	socklen_t addrlen = sizeof(sk_addr);

	TEST_ERRNO(connect(sk_listen, psaddr, addrlen), EISCONN);

	TEST_ERRNO(connect(sk_connected, psaddr, addrlen), 0);

	TEST_ERRNO(connect(sk_connected, psaddr, addrlen), EISCONN);

	TEST_ERRNO(connect(sk_accepted, psaddr, addrlen), EISCONN);
}
END_TEST()

FN_TEST(async_connect)
{
	struct pollfd pfd = { .fd = sk_bound, .events = POLLOUT };
	int err;
	socklen_t errlen;

	sk_addr.sin_port = 0xbeef;

#define ASYNC_CONNECT                                             \
	TEST_ERRNO(connect(sk_bound, (struct sockaddr *)&sk_addr, \
			   sizeof(sk_addr)),                      \
		   EINPROGRESS);                                  \
	TEST_RES(poll(&pfd, 1, 60),                               \
		 pfd.revents == (POLLOUT | POLLHUP | POLLERR));

	ASYNC_CONNECT;

	// `getpeername` will fail with `ENOTCONN` even before the second `connect`.
	errlen = sizeof(sk_addr);
	TEST_ERRNO(getpeername(sk_bound, (struct sockaddr *)&sk_addr, &errlen),
		   ENOTCONN);

	// The second `connect` will fail with `ECONNREFUSED`.
	TEST_ERRNO(connect(sk_bound, (struct sockaddr *)&sk_addr,
			   sizeof(sk_addr)),
		   ECONNREFUSED);

	ASYNC_CONNECT;

	// Reading the socket error will cause it to be cleared
	errlen = sizeof(err);
	TEST_RES(getsockopt(sk_bound, SOL_SOCKET, SO_ERROR, &err, &errlen),
		 errlen == sizeof(err) && err == ECONNREFUSED);
	TEST_RES(getsockopt(sk_bound, SOL_SOCKET, SO_ERROR, &err, &errlen),
		 errlen == sizeof(err) && err == 0);
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLOUT | POLLHUP));

	// `listen` won't succeed until the second `connect`.
	TEST_ERRNO(listen(sk_bound, 10), EINVAL);

	// The second `connect` will fail with `ECONNABORTED` if the socket
	// error is cleared.
	TEST_ERRNO(connect(sk_bound, (struct sockaddr *)&sk_addr,
			   sizeof(sk_addr)),
		   ECONNABORTED);

	ASYNC_CONNECT;

	// Testing `send` behavior before and after the second `connect`.
	TEST_ERRNO(send(sk_bound, &err, 0, 0), ECONNREFUSED);
	TEST_ERRNO(send(sk_bound, &err, 0, 0), EPIPE);
	TEST_ERRNO(connect(sk_bound, (struct sockaddr *)&sk_addr,
			   sizeof(sk_addr)),
		   ECONNABORTED);
	TEST_ERRNO(send(sk_bound, &err, 0, 0), EPIPE);

	ASYNC_CONNECT;

	// Testing `recv` behavior before and after the second `connect`.
	TEST_ERRNO(recv(sk_bound, &err, 0, 0), ECONNREFUSED);
	TEST_RES(recv(sk_bound, &err, 0, 0), _ret == 0);
	TEST_ERRNO(connect(sk_bound, (struct sockaddr *)&sk_addr,
			   sizeof(sk_addr)),
		   ECONNABORTED);
	TEST_ERRNO(recv(sk_bound, &err, 0, 0), ENOTCONN);

#undef ASYNC_CONNECT
}
END_TEST()

static void set_blocking(int sockfd, int is_blocking)
{
	int flags = CHECK(fcntl(sockfd, F_GETFL, 0));

	if (is_blocking) {
		flags &= ~O_NONBLOCK;
	} else {
		flags |= O_NONBLOCK;
	}

	CHECK(fcntl(sockfd, F_SETFL, flags));
}

FN_SETUP(enter_blocking_mode)
{
	set_blocking(sk_connected, 1);
	set_blocking(sk_bound, 1);
}
END_SETUP()

FN_TEST(sendmsg_and_recvmsg)
{
	struct msghdr msg = { 0 };
	struct iovec iov[2];
	char *message = "Message:";
	char *message2 = "Hello";
	iov[0].iov_base = message;
	iov[0].iov_len = strlen(message);
	iov[1].iov_base = message2;
	iov[1].iov_len = strlen(message2);
	msg.msg_iov = iov;
	msg.msg_iovlen = 2;

	// TEST CASE 1: Send one message and recv one message

	TEST_RES(sendmsg(sk_connected, &msg, 0),
		 _ret == strlen(message) + strlen(message2));

#define BUFFER_SIZE 50
	char concatenated[BUFFER_SIZE] = { 0 };
	strcat(concatenated, message);
	strcat(concatenated, message2);

	char buffer[BUFFER_SIZE] = { 0 };
	iov[0].iov_base = buffer;
	iov[0].iov_len = BUFFER_SIZE;
	msg.msg_iovlen = 1;

	TEST_RES(recvmsg(sk_accepted, &msg, 0),
		 _ret == strlen(concatenated) &&
			 strcmp(buffer, concatenated) == 0);

	// TEST CASE 2: Send two message and receive two message

	iov[0].iov_base = message;
	iov[0].iov_len = strlen(message);
	msg.msg_iovlen = 1;
	TEST_RES(sendmsg(sk_accepted, &msg, 0), _ret == strlen(message));
	TEST_RES(sendmsg(sk_accepted, &msg, 0), _ret == strlen(message));

	char first_buffer[BUFFER_SIZE] = { 0 };
	char second_buffer[BUFFER_SIZE] = { 0 };
	iov[0].iov_base = first_buffer;
	iov[0].iov_len = BUFFER_SIZE;
	iov[1].iov_base = second_buffer;
	iov[1].iov_len = BUFFER_SIZE;
	msg.msg_iovlen = 2;

	// Ensure two messages are prepared for receiving
	sleep(1);

	TEST_RES(recvmsg(sk_connected, &msg, 0), _ret == strlen(message) * 2);

	// TEST CASE 3: Send via a partially bad send buffer

	char *good_buffer = "abc";
	char *bad_buffer = (char *)1;
	iov[0].iov_base = good_buffer;
	iov[0].iov_len = strlen(good_buffer);
	iov[1].iov_base = bad_buffer;
	iov[1].iov_len = 1;
	msg.msg_iov = iov;
	msg.msg_iovlen = 2;
	TEST_ERRNO(sendmsg(sk_accepted, &msg, 0), EFAULT);

	// TEST CASE 4: Receive via a partially bad receive buffer

	iov[0].iov_base = good_buffer;
	iov[0].iov_len = strlen(good_buffer);
	msg.msg_iov = iov;
	msg.msg_iovlen = 1;

	TEST_RES(sendmsg(sk_accepted, &msg, 0), _ret == strlen(good_buffer));

	sleep(1);

	char recv_buffer[4096] = { 0 };
	iov[0].iov_base = recv_buffer;
	iov[0].iov_len = 1;
	TEST_RES(recvmsg(sk_connected, &msg, 0), _ret == 1);

	iov[0].iov_base = recv_buffer;
	iov[0].iov_len = 1;
	iov[1].iov_base = (char *)1;
	iov[1].iov_len = 1;
	msg.msg_iovlen = 2;
	TEST_ERRNO(recvmsg(sk_connected, &msg, 0), EFAULT);

	iov[0].iov_base = recv_buffer;
	iov[0].iov_len = 4096;
	msg.msg_iovlen = 1;
	TEST_RES(recvmsg(sk_connected, &msg, 0),
		 _ret == strlen(good_buffer) - 1);

	// TEST CASE 5: Send a large buffer

	int big_buffer_size = 1000000;
	char *big_buffer = (char *)calloc(0, big_buffer_size);
	iov[0].iov_base = big_buffer;
	iov[0].iov_len = big_buffer_size;
	msg.msg_iovlen = 2;

	int sndbuf = 0;
	socklen_t optlen = sizeof(sndbuf);
	TEST_SUCC(getsockopt(sk_accepted, SOL_SOCKET, SO_SNDBUF, &sndbuf,
			     &optlen));
	TEST_RES(sendmsg(sk_accepted, &msg, 0), _ret <= sndbuf);
}
END_TEST()

FN_TEST(self_connect)
{
	int sk;
	char buf[5];

	sk = TEST_SUCC(socket(PF_INET, SOCK_STREAM, 0));

	sk_addr.sin_port = htons(8888);
	TEST_SUCC(bind(sk, (struct sockaddr *)&sk_addr, sizeof(sk_addr)));
	TEST_SUCC(connect(sk, (struct sockaddr *)&sk_addr, sizeof(sk_addr)));

	TEST_RES(write(sk, "hello", 5), _ret == 5);
	TEST_RES(read(sk, buf, 5), _ret == 5 && memcmp(buf, "hello", 5) == 0);

	TEST_SUCC(close(sk));
}
END_TEST()

FN_TEST(listen_at_the_same_address)
{
	int sk_listen1;
	int sk_listen2;

	sk_listen1 = TEST_SUCC(socket(PF_INET, SOCK_STREAM, 0));
	sk_listen2 = TEST_SUCC(socket(PF_INET, SOCK_STREAM, 0));

	int reuse_option = 1;
	TEST_SUCC(setsockopt(sk_listen1, SOL_SOCKET, SO_REUSEADDR,
			     &reuse_option, sizeof(reuse_option)));
	TEST_SUCC(setsockopt(sk_listen2, SOL_SOCKET, SO_REUSEADDR,
			     &reuse_option, sizeof(reuse_option)));

	sk_addr.sin_port = htons(8889);
	TEST_SUCC(
		bind(sk_listen1, (struct sockaddr *)&sk_addr, sizeof(sk_addr)));
	TEST_SUCC(
		bind(sk_listen2, (struct sockaddr *)&sk_addr, sizeof(sk_addr)));

	TEST_SUCC(listen(sk_listen1, 3));
	TEST_ERRNO(listen(sk_listen2, 3), EADDRINUSE);

	TEST_SUCC(close(sk_listen1));
	TEST_SUCC(close(sk_listen2));
}
END_TEST()

FN_TEST(bind_and_connect_same_address)
{
	int sk_listen;
	int sk_connect1;
	int sk_connect2;

	sk_listen = TEST_SUCC(socket(PF_INET, SOCK_STREAM, 0));
	sk_connect1 = TEST_SUCC(socket(PF_INET, SOCK_STREAM, 0));
	sk_connect2 = TEST_SUCC(socket(PF_INET, SOCK_STREAM, 0));

	int reuse_option = 1;
	TEST_SUCC(setsockopt(sk_connect1, SOL_SOCKET, SO_REUSEADDR,
			     &reuse_option, sizeof(reuse_option)));
	TEST_SUCC(setsockopt(sk_connect2, SOL_SOCKET, SO_REUSEADDR,
			     &reuse_option, sizeof(reuse_option)));

	int listen_port = 8890;
	int connect_port = 8891;
	sk_addr.sin_port = htons(listen_port);
	TEST_SUCC(
		bind(sk_listen, (struct sockaddr *)&sk_addr, sizeof(sk_addr)));
	sk_addr.sin_port = htons(connect_port);
	TEST_SUCC(bind(sk_connect1, (struct sockaddr *)&sk_addr,
		       sizeof(sk_addr)));
	TEST_SUCC(bind(sk_connect2, (struct sockaddr *)&sk_addr,
		       sizeof(sk_addr)));

	TEST_SUCC(listen(sk_listen, 3));

	// For blocking sockets, conflict addresses result in `EADDRNOTAVAIL`.
	sk_addr.sin_port = htons(listen_port);
	TEST_SUCC(connect(sk_connect1, (struct sockaddr *)&sk_addr,
			  sizeof(sk_addr)));
	TEST_ERRNO(connect(sk_connect2, (struct sockaddr *)&sk_addr,
			   sizeof(sk_addr)),
		   EADDRNOTAVAIL);

	// For non-blocking sockets, conflict addresses also result in `EADDRNOTAVAIL`.
	// (`EINPROGRESS` should _not_ be returned in this case.)
	set_blocking(sk_connect2, 0);
	TEST_ERRNO(connect(sk_connect2, (struct sockaddr *)&sk_addr,
			   sizeof(sk_addr)),
		   EADDRNOTAVAIL);

	TEST_SUCC(close(sk_listen));
	TEST_SUCC(close(sk_connect1));
	TEST_SUCC(close(sk_connect2));
}
END_TEST()

#define SETUP_CONN                                                 \
	sk_addr.sin_port = S_PORT;                                 \
                                                                   \
	sk_connect = TEST_SUCC(socket(PF_INET, SOCK_STREAM, 0));   \
	pfd.fd = sk_connect;                                       \
	TEST_SUCC(connect(sk_connect, (struct sockaddr *)&sk_addr, \
			  sizeof(sk_addr)));                       \
                                                                   \
	len = sizeof(sk_addr);                                     \
	sk_accept = TEST_SUCC(                                     \
		accept(sk_listen, (struct sockaddr *)&sk_addr, &len));

FN_TEST(shutdown_shutdown)
{
	int sk_accept;
	int sk_connect;
	socklen_t len;
	struct pollfd pfd __attribute__((unused));

	SETUP_CONN;

	// Test 1: Perform `shutdown` multiple times
	TEST_SUCC(shutdown(sk_accept, SHUT_RDWR));
	TEST_SUCC(shutdown(sk_accept, SHUT_RDWR));

	// Test 2: Perform `shutdown` after the connection is closed
	TEST_SUCC(shutdown(sk_connect, SHUT_RDWR));
	TEST_ERRNO(shutdown(sk_connect, SHUT_RD), ENOTCONN);
	TEST_ERRNO(shutdown(sk_connect, SHUT_WR), ENOTCONN);
	TEST_ERRNO(shutdown(sk_accept, SHUT_RD), ENOTCONN);
	TEST_ERRNO(shutdown(sk_accept, SHUT_WR), ENOTCONN);

	TEST_SUCC(close(sk_accept));
	TEST_SUCC(close(sk_connect));
}
END_TEST()

FN_TEST(connreset)
{
	int sk_accept;
	int sk_connect;
	struct linger lin = { .l_onoff = 1, .l_linger = 0 };
	struct pollfd pfd = { .events = POLLIN | POLLOUT };
	char buf[6] = "hello";
	int err;
	socklen_t len;

#define RESET_CONN                                                   \
	TEST_SUCC(setsockopt(sk_accept, SOL_SOCKET, SO_LINGER, &lin, \
			     sizeof(lin)));                          \
	TEST_SUCC(close(sk_accept));

#define EV_ERR (POLLIN | POLLOUT | POLLHUP | POLLERR)
#define EV_NO_ERR (POLLIN | POLLOUT | POLLHUP)

	// Test 1: `recv` should fail with `ECONNRESET`

	SETUP_CONN;
	RESET_CONN;

	TEST_RES(poll(&pfd, 1, 0), pfd.revents == EV_ERR);
	TEST_ERRNO(recv(sk_connect, buf, 0, 0), ECONNRESET);
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == EV_NO_ERR);

	TEST_RES(recv(sk_connect, buf, 0, 0), _ret == 0);
	TEST_SUCC(close(sk_connect));

	// Test 2: `send` should fail with `ECONNRESET`

	SETUP_CONN;
	RESET_CONN;

	TEST_RES(poll(&pfd, 1, 0), pfd.revents == EV_ERR);
	TEST_ERRNO(send(sk_connect, buf, 0, 0), ECONNRESET);
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == EV_NO_ERR);

	TEST_ERRNO(send(sk_connect, buf, 0, 0), EPIPE);
	TEST_SUCC(close(sk_connect));

	// Test 3: `recv` should drain the buffer, then fail with `ECONNRESET`

	SETUP_CONN;
	TEST_RES(send(sk_accept, buf, sizeof(buf), 0), _ret == sizeof(buf));
	RESET_CONN;

	TEST_RES(recv(sk_connect, buf, 4, 0),
		 _ret == 4 && memcmp(buf, "hell", 4) == 0);
	TEST_RES(recv(sk_connect, buf, sizeof(buf), 0),
		 _ret == 2 && memcmp(buf, "o", 2) == 0);
	TEST_ERRNO(recv(sk_connect, buf, sizeof(buf), 0), ECONNRESET);

	TEST_RES(recv(sk_connect, buf, 0, 0), _ret == 0);
	TEST_SUCC(close(sk_connect));

	// Test 3: `getsockopt(SO_ERROR)` should report `ECONNRESET`

	SETUP_CONN;
	RESET_CONN;

	TEST_RES(poll(&pfd, 1, 0), pfd.revents == EV_ERR);
	len = sizeof(err);
	TEST_RES(getsockopt(sk_connect, SOL_SOCKET, SO_ERROR, &err, &len),
		 len == sizeof(err) && err == ECONNRESET);
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == EV_NO_ERR);

	TEST_RES(getsockopt(sk_connect, SOL_SOCKET, SO_ERROR, &err, &len),
		 len == sizeof(err) && err == 0);
	TEST_SUCC(close(sk_connect));

#undef EV_ERR
#undef EV_NO_ERR

#undef RESET_CONN
}
END_TEST()

#undef SETUP_CONN

FN_TEST(listen_close)
{
	int sk_listen;
	int sk_connect;

	sk_addr.sin_port = htons(0x4321);

	sk_listen = TEST_SUCC(socket(PF_INET, SOCK_STREAM, 0));
	TEST_SUCC(
		bind(sk_listen, (struct sockaddr *)&sk_addr, sizeof(sk_addr)));
	TEST_SUCC(listen(sk_listen, 10));

	sk_connect = TEST_SUCC(socket(PF_INET, SOCK_STREAM, 0));
	TEST_SUCC(connect(sk_connect, (struct sockaddr *)&sk_addr,
			  sizeof(sk_addr)));

	// Test: `close(sk_listen)` will reset all connections in the backlog
	TEST_SUCC(close(sk_listen));
	TEST_ERRNO(send(sk_connect, &sk_connect, sizeof(sk_connect), 0),
		   ECONNRESET);

	TEST_SUCC(close(sk_connect));
}
END_TEST()
