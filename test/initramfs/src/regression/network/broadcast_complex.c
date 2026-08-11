// SPDX-License-Identifier: MPL-2.0

#include <sys/socket.h>
#include <netinet/ip.h>
#include <arpa/inet.h>
#include <unistd.h>

#include "../common/test.h"

static struct sockaddr_in broadcast_port1; // 127.255.255.255:12345
static struct sockaddr_in broadcast_port2; // 127.255.255.255:12346
static struct sockaddr_in localhost_port1; // 127.0.0.1:12345
static struct sockaddr_in localhost_port2; // 127.0.0.1:12346

static char msg[16] = "hello world";
static char buf[16];
static struct sockaddr_in recv_addr;
static socklen_t recv_addrlen;

FN_SETUP(init)
{
	broadcast_port1.sin_family = AF_INET;
	CHECK(inet_aton("127.255.255.255", &broadcast_port1.sin_addr));
	broadcast_port1.sin_port = htons(12345);

	broadcast_port2.sin_family = AF_INET;
	CHECK(inet_aton("127.255.255.255", &broadcast_port2.sin_addr));
	broadcast_port2.sin_port = htons(12346);

	localhost_port1.sin_family = AF_INET;
	CHECK(inet_aton("127.0.0.1", &localhost_port1.sin_addr));
	localhost_port1.sin_port = htons(12345);

	localhost_port2.sin_family = AF_INET;
	CHECK(inet_aton("127.0.0.1", &localhost_port2.sin_addr));
	localhost_port2.sin_port = htons(12346);
}
END_SETUP()

static int new_udp(struct sockaddr_in *addr)
{
	int sk;
	int opt_one = 1;

	sk = socket(AF_INET, SOCK_DGRAM | SOCK_NONBLOCK, 0);
	if (sk < 0)
		return sk;

	CHECK(setsockopt(sk, SOL_SOCKET, SO_REUSEADDR, &opt_one,
			 sizeof(opt_one)));
	CHECK(setsockopt(sk, SOL_SOCKET, SO_BROADCAST, &opt_one,
			 sizeof(opt_one)));

	CHECK(bind(sk, (struct sockaddr *)addr, sizeof(struct sockaddr_in)));

	return sk;
}

#define TEST_SEND_TO(fd, addr)                                             \
	TEST_RES(sendto(fd, msg, sizeof(msg), 0, (struct sockaddr *)&addr, \
			sizeof(addr)),                                     \
		 _ret == sizeof(msg));

#define TEST_RECV_FROM(fd, addr)                                              \
	memset(buf, 0, sizeof(buf));                                          \
	recv_addrlen = sizeof(struct sockaddr);                               \
	TEST_RES(recvfrom(fd, buf, sizeof(buf), 0,                            \
			  (struct sockaddr *)&recv_addr, &recv_addrlen),      \
		 _ret == sizeof(msg) && memcmp(buf, msg, sizeof(msg)) == 0 && \
			 recv_addrlen == sizeof(addr) &&                      \
			 memcmp(&recv_addr, &addr, sizeof(addr)) == 0)

#define TEST_RECV_EAGAIN(fd)                                               \
	recv_addrlen = sizeof(struct sockaddr);                            \
	TEST_ERRNO(recvfrom(fd, buf, sizeof(buf), 0,                       \
			    (struct sockaddr *)&recv_addr, &recv_addrlen), \
		   EAGAIN)

FN_TEST(udp_broadcast)
{
	int sender, receiver0, receiver1, receiver2, receiver3;

	sender = TEST_SUCC(new_udp(&localhost_port1));
	receiver0 = TEST_SUCC(new_udp(&broadcast_port1));
	receiver1 = TEST_SUCC(new_udp(&broadcast_port1));
	receiver2 = TEST_SUCC(new_udp(&broadcast_port2));
	receiver3 = TEST_SUCC(new_udp(&localhost_port1));

	// Broadcast packets can be received by multiple sockets.
	TEST_SEND_TO(sender, broadcast_port1);
	TEST_RECV_EAGAIN(sender);
	TEST_RECV_FROM(receiver0, localhost_port1);
	TEST_RECV_FROM(receiver1, localhost_port1);
	TEST_RECV_EAGAIN(receiver2);
	TEST_RECV_EAGAIN(receiver3);

	TEST_SUCC(close(sender));
	sender = TEST_SUCC(new_udp(&broadcast_port1));

	// Source addresses can never be the broadcast address.
	TEST_SEND_TO(sender, broadcast_port1);
	TEST_RECV_FROM(sender, localhost_port1);
	TEST_RECV_FROM(receiver0, localhost_port1);
	TEST_RECV_FROM(receiver1, localhost_port1);
	TEST_RECV_EAGAIN(receiver2);
	TEST_RECV_EAGAIN(receiver3);

	TEST_SUCC(close(sender));
	TEST_SUCC(close(receiver0));
	TEST_SUCC(close(receiver1));
	TEST_SUCC(close(receiver2));
	TEST_SUCC(close(receiver3));
}
END_TEST()

#define CALL_SOCKADDR_OP(op, sk, addr) \
	op(sk, (struct sockaddr *)&addr, sizeof(addr))

#define TEST_SOCK_NAME(sk, addr)                                \
	recv_addrlen = sizeof(recv_addr);                       \
	TEST_RES(getsockname(sk, (struct sockaddr *)&recv_addr, \
			     &recv_addrlen),                    \
		 recv_addrlen == sizeof(addr) &&                \
			 memcmp(&recv_addr, &addr, sizeof(addr)) == 0)

FN_TEST(tcp_broadcast_bind)
{
	int sk1, sk2;

	sk1 = TEST_SUCC(socket(AF_INET, SOCK_STREAM, 0));
	sk2 = TEST_SUCC(socket(AF_INET, SOCK_STREAM, 0));

	TEST_SUCC(CALL_SOCKADDR_OP(bind, sk1, broadcast_port2));
	TEST_SOCK_NAME(sk1, broadcast_port2);

	TEST_ERRNO(CALL_SOCKADDR_OP(bind, sk2, broadcast_port2), EADDRINUSE);
	TEST_SUCC(CALL_SOCKADDR_OP(bind, sk2, localhost_port2));

	// TCP sockets can bind to broadcast addresses. However, they fall back
	// to interface addresses when connecting.
	TEST_ERRNO(CALL_SOCKADDR_OP(connect, sk1, localhost_port1),
		   ECONNREFUSED);
	TEST_SOCK_NAME(sk1, localhost_port2);
	TEST_SOCK_NAME(sk2, localhost_port2);

	TEST_SUCC(close(sk2));
	sk2 = TEST_SUCC(socket(AF_INET, SOCK_STREAM, 0));

	// New sockets cannot bind to the address to which `connect` falls back.
	TEST_ERRNO(CALL_SOCKADDR_OP(bind, sk2, localhost_port2), EADDRINUSE);
	TEST_SUCC(CALL_SOCKADDR_OP(bind, sk2, broadcast_port2));

	TEST_SUCC(close(sk1));
	TEST_SUCC(close(sk2));
}
END_TEST()

FN_TEST(tcp_broadcast_listen)
{
	int sk1, sk2;

	sk1 = TEST_SUCC(socket(AF_INET, SOCK_STREAM, 0));
	sk2 = TEST_SUCC(socket(AF_INET, SOCK_STREAM, 0));

	TEST_SUCC(CALL_SOCKADDR_OP(bind, sk1, broadcast_port1));
	TEST_SUCC(listen(sk1, 1));

	// TCP sockets cannot connect to broadcast addresses.
	TEST_SUCC(CALL_SOCKADDR_OP(bind, sk2, localhost_port1));
	TEST_ERRNO(CALL_SOCKADDR_OP(connect, sk2, broadcast_port1),
		   ENETUNREACH);

	TEST_SUCC(close(sk1));
	TEST_SUCC(close(sk2));
}
END_TEST()
