// SPDX-License-Identifier: MPL-2.0

#include <unistd.h>
#include <sys/signal.h>
#include <sys/socket.h>
#include <sys/poll.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <fcntl.h>

#include "test.h"

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

	TEST_ERRNO(send(sk_bound, buf, 1, 0), EPIPE);

	TEST_ERRNO(send(sk_listen, buf, 1, 0), EPIPE);
}
END_TEST()

FN_TEST(recv)
{
	char buf[1] = { 'z' };

	TEST_ERRNO(recv(sk_unbound, buf, 1, 0), ENOTCONN);

	TEST_ERRNO(recv(sk_bound, buf, 1, 0), ENOTCONN);

	TEST_ERRNO(recv(sk_listen, buf, 1, 0), ENOTCONN);
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
	socklen_t errlen = sizeof(err);

	sk_addr.sin_port = 0xbeef;
	TEST_ERRNO(connect(sk_bound, (struct sockaddr *)&sk_addr,
			   sizeof(sk_addr)),
		   EINPROGRESS);

	TEST_RES(poll(&pfd, 1, 60), pfd.revents & POLLOUT);

	TEST_RES(getsockopt(sk_bound, SOL_SOCKET, SO_ERROR, &err, &errlen),
		 errlen == sizeof(err) && err == ECONNREFUSED);

	// Reading the socket error will cause it to be cleared
	TEST_RES(getsockopt(sk_bound, SOL_SOCKET, SO_ERROR, &err, &errlen),
		 errlen == sizeof(err) && err == 0);
}
END_TEST()

void set_blocking(int sockfd)
{
	int flags = CHECK(fcntl(sockfd, F_GETFL, 0));
	CHECK(fcntl(sockfd, F_SETFL, flags & (~O_NONBLOCK)));
}

FN_SETUP(enter_blocking_mode)
{
	set_blocking(sk_connected);
	set_blocking(sk_bound);
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
