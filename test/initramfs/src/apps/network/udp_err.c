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
static int sk_connected;

FN_SETUP(unbound)
{
	sk_unbound = CHECK(socket(PF_INET, SOCK_DGRAM | SOCK_NONBLOCK, 0));
}
END_SETUP()

FN_SETUP(bound)
{
	sk_bound = CHECK(socket(PF_INET, SOCK_DGRAM | SOCK_NONBLOCK, 0));

	sk_addr.sin_port = C_PORT;
	CHECK(bind(sk_bound, (struct sockaddr *)&sk_addr, sizeof(sk_addr)));
}
END_SETUP()

FN_SETUP(connected)
{
	sk_connected = CHECK(socket(PF_INET, SOCK_DGRAM | SOCK_NONBLOCK, 0));

	sk_addr.sin_port = C_PORT;
	CHECK(connect(sk_connected, (struct sockaddr *)&sk_addr,
		      sizeof(sk_addr)));
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

	TEST_RES(getsockname(sk_connected, psaddr, &addrlen),
		 addrlen == sizeof(saddr) && saddr.sin_port != C_PORT);
}
END_TEST()

FN_TEST(getpeername)
{
	struct sockaddr_in saddr = { .sin_port = 0xbeef };
	struct sockaddr *psaddr = (struct sockaddr *)&saddr;
	socklen_t addrlen = sizeof(saddr);

	TEST_ERRNO(getpeername(sk_unbound, psaddr, &addrlen), ENOTCONN);

	TEST_ERRNO(getpeername(sk_bound, psaddr, &addrlen), ENOTCONN);

	TEST_RES(getpeername(sk_connected, psaddr, &addrlen),
		 addrlen == sizeof(saddr) && saddr.sin_port == C_PORT);
}
END_TEST()

FN_TEST(send)
{
	char buf[1] = { 'z' };

	TEST_ERRNO(send(sk_unbound, buf, 1, 0), EDESTADDRREQ);
	TEST_ERRNO(send(sk_unbound, buf, 0, 0), EDESTADDRREQ);
	TEST_ERRNO(write(sk_unbound, buf, 1), EDESTADDRREQ);
	TEST_ERRNO(write(sk_unbound, buf, 0), EDESTADDRREQ);

	TEST_ERRNO(send(sk_bound, buf, 1, 0), EDESTADDRREQ);
	TEST_ERRNO(send(sk_bound, buf, 0, 0), EDESTADDRREQ);
	TEST_ERRNO(write(sk_bound, buf, 1), EDESTADDRREQ);
	TEST_ERRNO(write(sk_bound, buf, 0), EDESTADDRREQ);
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

	TEST_ERRNO(recv(sk_connected, buf, 1, 0), EAGAIN);
	TEST_ERRNO(recv(sk_connected, buf, 0, 0), EAGAIN);
	TEST_ERRNO(read(sk_connected, buf, 1), EAGAIN);
	TEST_SUCC(read(sk_connected, buf, 0));
}
END_TEST()

FN_TEST(send_and_recv)
{
	char buf[1];
	struct sockaddr_in saddr;
	socklen_t addrlen = sizeof(saddr);

	sk_addr.sin_port = C_PORT;
	buf[0] = 'a';
	TEST_RES(sendto(sk_bound, buf, 1, 0, (struct sockaddr *)&sk_addr,
			sizeof(sk_addr)),
		 _ret == 1);

	buf[0] = 'b';
	TEST_RES(send(sk_connected, buf, 1, 0), _ret == 1);

	saddr.sin_port = 0;
	buf[0] = 0;
	TEST_RES(recvfrom(sk_bound, buf, 1, 0, (struct sockaddr *)&saddr,
			  &addrlen),
		 _ret == 1 && addrlen == sizeof(sk_addr) &&
			 saddr.sin_port == C_PORT && buf[0] == 'a');

	saddr.sin_port = 0;
	buf[0] = 0;
	TEST_RES(recvfrom(sk_bound, buf, 1, 0, (struct sockaddr *)&saddr,
			  &addrlen),
		 _ret == 1 && addrlen == sizeof(sk_addr) &&
			 saddr.sin_port != C_PORT && buf[0] == 'b');
}
END_TEST()

FN_TEST(bind)
{
	struct sockaddr *psaddr = (struct sockaddr *)&sk_addr;
	socklen_t addrlen = sizeof(sk_addr);

	TEST_ERRNO(bind(sk_unbound, psaddr, addrlen - 1), EINVAL);

	TEST_ERRNO(bind(sk_bound, psaddr, addrlen), EINVAL);

	TEST_ERRNO(bind(sk_connected, psaddr, addrlen), EINVAL);
}
END_TEST()

FN_TEST(bind_reuseaddr)
{
	sk_addr.sin_port = htons(8081);
	struct sockaddr *psaddr = (struct sockaddr *)&sk_addr;
	socklen_t addrlen = sizeof(sk_addr);

	int disable = 0;
	int enable = 1;
	int sk1 = TEST_SUCC(socket(PF_INET, SOCK_DGRAM | SOCK_NONBLOCK, 0));
	int sk2 = TEST_SUCC(socket(PF_INET, SOCK_DGRAM | SOCK_NONBLOCK, 0));

	TEST_SUCC(bind(sk1, psaddr, addrlen));

	TEST_ERRNO(bind(sk2, psaddr, addrlen), EADDRINUSE);

	TEST_SUCC(setsockopt(sk1, SOL_SOCKET, SO_REUSEADDR, &disable,
			     sizeof(disable)));
	TEST_SUCC(setsockopt(sk2, SOL_SOCKET, SO_REUSEADDR, &enable,
			     sizeof(enable)));
	TEST_ERRNO(bind(sk2, psaddr, addrlen), EADDRINUSE);

	TEST_SUCC(setsockopt(sk1, SOL_SOCKET, SO_REUSEADDR, &enable,
			     sizeof(enable)));
	TEST_SUCC(setsockopt(sk2, SOL_SOCKET, SO_REUSEADDR, &disable,
			     sizeof(disable)));
	TEST_ERRNO(bind(sk2, psaddr, addrlen), EADDRINUSE);

	TEST_SUCC(setsockopt(sk1, SOL_SOCKET, SO_REUSEADDR, &disable,
			     sizeof(disable)));
	TEST_SUCC(setsockopt(sk2, SOL_SOCKET, SO_REUSEADDR, &enable,
			     sizeof(enable)));
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
	TEST_ERRNO(listen(sk_unbound, 2), EOPNOTSUPP);

	TEST_ERRNO(listen(sk_bound, 2), EOPNOTSUPP);

	TEST_ERRNO(listen(sk_connected, 2), EOPNOTSUPP);
}
END_TEST()

FN_TEST(accept)
{
	struct sockaddr_in saddr;
	struct sockaddr *psaddr = (struct sockaddr *)&saddr;
	socklen_t addrlen = sizeof(saddr);

	TEST_ERRNO(accept(sk_unbound, psaddr, &addrlen), EOPNOTSUPP);

	TEST_ERRNO(accept(sk_bound, psaddr, &addrlen), EOPNOTSUPP);

	TEST_ERRNO(accept(sk_connected, psaddr, &addrlen), EOPNOTSUPP);
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

	pfd.fd = sk_connected;
	TEST_RES(poll(&pfd, 1, 0),
		 (pfd.revents & (POLLIN | POLLOUT)) == POLLOUT);
}
END_TEST()

FN_TEST(connect)
{
	struct sockaddr *psaddr = (struct sockaddr *)&sk_addr;
	socklen_t addrlen = sizeof(sk_addr);

	TEST_SUCC(connect(sk_connected, psaddr, addrlen));
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
	struct sockaddr_in saddr;
	socklen_t addrlen = sizeof(saddr);

	sk_addr.sin_port = C_PORT;

	struct msghdr msg = { 0 };
	struct iovec iov[1];
	char *message = "Message";
	iov[0].iov_base = message;
	iov[0].iov_len = strlen(message);
	msg.msg_iov = iov;
	msg.msg_iovlen = 1;
	msg.msg_name = (struct sockaddr *)&sk_addr;
	msg.msg_namelen = addrlen;

	// TEST CASE 1: Send one message and receive one message

	TEST_RES(sendmsg(sk_connected, &msg, 0), _ret == strlen(message));

#define BUFFER_SIZE 50
	char buffer[BUFFER_SIZE];
	iov[0].iov_base = buffer;
	iov[0].iov_len = BUFFER_SIZE;
	msg.msg_name = 0;
	TEST_RES(recvmsg(sk_bound, &msg, 0),
		 _ret == strlen(message) && strcmp(message, buffer) == 0);

	// TEST CASE 2: Send two messages and receive two messages

	iov[0].iov_base = message;
	iov[0].iov_len = strlen(message);
	msg.msg_name = (struct sockaddr *)&sk_addr;
	msg.msg_namelen = addrlen;

	TEST_RES(sendmsg(sk_connected, &msg, 0), _ret == strlen(message));
	TEST_RES(sendmsg(sk_connected, &msg, 0), _ret == strlen(message));

	iov[0].iov_base = buffer;
	iov[0].iov_len = BUFFER_SIZE;

	TEST_RES(recvmsg(sk_bound, &msg, 0),
		 _ret == strlen(message) && strcmp(message, buffer) == 0);
	TEST_RES(recvmsg(sk_bound, &msg, 0),
		 _ret == strlen(message) && strcmp(message, buffer) == 0);
}
END_TEST()

void set_nonblocking(int sockfd)
{
	int flags = CHECK(fcntl(sockfd, F_GETFL, 0));
	CHECK(fcntl(sockfd, F_SETFL, flags | O_NONBLOCK));
}

FN_SETUP(enter_nonblocking_mode)
{
	set_nonblocking(sk_connected);
	set_nonblocking(sk_bound);
}
END_SETUP()

FN_TEST(sendmsg_and_recvmsg_bad_buffer)
{
	struct sockaddr_in saddr;
	socklen_t addrlen = sizeof(saddr);

	sk_addr.sin_port = C_PORT;

	struct msghdr msg = { 0 };
	struct iovec iov[2];
	msg.msg_name = (struct sockaddr *)&sk_addr;
	msg.msg_namelen = addrlen;

	// TEST CASE 1: Send via a partially bad send buffer

	char *good_buffer = "abc";
	char *bad_buffer = (char *)1;
	iov[0].iov_base = good_buffer;
	iov[0].iov_len = strlen(good_buffer);
	iov[1].iov_base = bad_buffer;
	iov[1].iov_len = 1;
	msg.msg_iov = iov;
	msg.msg_iovlen = 2;
	TEST_ERRNO(sendmsg(sk_connected, &msg, 0), EFAULT);

	// TEST CASE 2: Receive via a partially bad receive buffer

	iov[0].iov_base = good_buffer;
	iov[0].iov_len = strlen(good_buffer);
	msg.msg_iov = iov;
	msg.msg_iovlen = 1;

	TEST_RES(sendmsg(sk_connected, &msg, 0), _ret == strlen(good_buffer));
	TEST_RES(sendmsg(sk_connected, &msg, 0), _ret == strlen(good_buffer));
	TEST_RES(sendmsg(sk_connected, &msg, 0), _ret == strlen(good_buffer));
	TEST_RES(sendmsg(sk_connected, &msg, 0), _ret == strlen(good_buffer));

	sleep(1);

	char recv_buffer[4096] = { 0 };
	iov[0].iov_base = recv_buffer;
	iov[0].iov_len = 1;
	TEST_RES(recvmsg(sk_bound, &msg, 0), _ret == 1);

	iov[0].iov_base = recv_buffer;
	iov[0].iov_len = 1;
	iov[1].iov_base = (char *)1;
	iov[1].iov_len = 1;
	msg.msg_iovlen = 2;
	TEST_ERRNO(recvmsg(sk_bound, &msg, 0), EFAULT);

	iov[0].iov_base = recv_buffer;
	iov[0].iov_len = 4096;
	msg.msg_iovlen = 1;
	TEST_RES(recvmsg(sk_bound, &msg, 0), _ret == strlen(good_buffer));

	iov[0].iov_base = recv_buffer;
	iov[0].iov_len = 4096;
	iov[1].iov_base = (char *)1;
	iov[1].iov_len = 1;
	msg.msg_iovlen = 2;
	TEST_RES(recvmsg(sk_bound, &msg, 0), _ret == strlen(good_buffer));
}
END_TEST()

FN_TEST(self_connect)
{
	int sk;
	char buf[5];

	sk = TEST_SUCC(socket(PF_INET, SOCK_DGRAM, 0));

	sk_addr.sin_port = htons(7777);
	TEST_SUCC(bind(sk, (struct sockaddr *)&sk_addr, sizeof(sk_addr)));
	TEST_SUCC(connect(sk, (struct sockaddr *)&sk_addr, sizeof(sk_addr)));

	TEST_RES(write(sk, "hello", 5), _ret == 5);
	TEST_RES(read(sk, buf, 5), _ret == 5 && memcmp(buf, "hello", 5) == 0);

	TEST_SUCC(close(sk));
}
END_TEST()
