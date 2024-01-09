// SPDX-License-Identifier: MPL-2.0

#include <unistd.h>
#include <sys/signal.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>

#include "test.h"

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
	socklen_t addrlen = sizeof(saddr);

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

	TEST_ERRNO(send(sk_bound, buf, 1, 0), EDESTADDRREQ);
}
END_TEST()

FN_TEST(recv)
{
	char buf[1] = { 'z' };

	TEST_ERRNO(recv(sk_unbound, buf, 1, 0), EAGAIN);

	TEST_ERRNO(recv(sk_bound, buf, 1, 0), EAGAIN);

	TEST_ERRNO(recv(sk_connected, buf, 1, 0), EAGAIN);
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
