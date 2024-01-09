// SPDX-License-Identifier: MPL-2.0

#include <unistd.h>
#include <sys/signal.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>

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
		   _ret == 0 || errno == EINPROGRESS);
}
END_SETUP()

FN_SETUP(accpected)
{
	struct sockaddr addr;
	socklen_t addrlen = sizeof(addr);

	do {
		sk_accepted = CHECK_WITH(accept(sk_listen, &addr, &addrlen),
					 _ret >= 0 || errno == EAGAIN);
	} while (sk_accepted < 0);
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

	TEST_ERRNO(bind(sk_bound, psaddr, addrlen), EINVAL);

	TEST_ERRNO(bind(sk_listen, psaddr, addrlen), EINVAL);

	TEST_ERRNO(bind(sk_connected, psaddr, addrlen), EINVAL);

	TEST_ERRNO(bind(sk_accepted, psaddr, addrlen), EINVAL);
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
