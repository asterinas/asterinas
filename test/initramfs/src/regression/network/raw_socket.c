// SPDX-License-Identifier: MPL-2.0

#include <unistd.h>
#include <sys/socket.h>
#include <sys/poll.h>
#include <netinet/in.h>
#include <netinet/ip.h>
#include <arpa/inet.h>
#include <fcntl.h>
#include <string.h>
#include <stdlib.h>
#include <stdint.h>

#include "../common/test.h"

static struct sockaddr_in loopback_addr;

#define TEST_ADDR htons(0x1234)

FN_SETUP(general)
{
	loopback_addr.sin_family = AF_INET;
	loopback_addr.sin_port = TEST_ADDR;
	CHECK(inet_aton("127.0.0.1", &loopback_addr.sin_addr));
}
END_SETUP()

static int sk_raw_icmp;
static int sk_raw_udp;
static int sk_unbound;

FN_SETUP(raw_icmp_bound)
{
	sk_raw_icmp =
		CHECK(socket(AF_INET, SOCK_RAW | SOCK_NONBLOCK, IPPROTO_ICMP));

	CHECK(bind(sk_raw_icmp, (struct sockaddr *)&loopback_addr,
		   sizeof(loopback_addr)));
}
END_SETUP()

FN_SETUP(raw_udp_bound)
{
	sk_raw_udp =
		CHECK(socket(AF_INET, SOCK_RAW | SOCK_NONBLOCK, IPPROTO_UDP));

	CHECK(bind(sk_raw_udp, (struct sockaddr *)&loopback_addr,
		   sizeof(loopback_addr)));
}
END_SETUP()

FN_SETUP(raw_icmp_unbound)
{
	sk_unbound =
		CHECK(socket(AF_INET, SOCK_RAW | SOCK_NONBLOCK, IPPROTO_ICMP));
}
END_SETUP()

FN_TEST(socket_no_cap)
{
	int sk = socket(AF_INET, SOCK_RAW, IPPROTO_ICMP);
	if (sk < 0) {
		TEST_ERRNO(socket(AF_INET, SOCK_RAW, IPPROTO_ICMP), EPERM);
	} else {
		/* we are root, so this succeeds */
		TEST_RES(sk, sk >= 0);
		close(sk);
	}
}
END_TEST()

FN_TEST(socket_create)
{
	int sk1 = TEST_SUCC(
		socket(AF_INET, SOCK_RAW | SOCK_NONBLOCK, IPPROTO_ICMP));
	int sk2 = TEST_SUCC(
		socket(AF_INET, SOCK_RAW | SOCK_NONBLOCK, IPPROTO_IGMP));
	int sk3 = TEST_SUCC(
		socket(AF_INET, SOCK_RAW | SOCK_NONBLOCK, IPPROTO_RAW));

	TEST_SUCC(close(sk1));
	TEST_SUCC(close(sk2));
	TEST_SUCC(close(sk3));
}
END_TEST()

FN_TEST(socket_unsupported_protocol)
{
	/* IPPROTO_MAX is not a valid protocol */
	TEST_ERRNO(socket(AF_INET, SOCK_RAW, 253), EINVAL);
}
END_TEST()

FN_TEST(bind_twice_same_protocol)
{
	int sk1 = TEST_SUCC(
		socket(AF_INET, SOCK_RAW | SOCK_NONBLOCK, IPPROTO_ICMP));
	int sk2 = TEST_SUCC(
		socket(AF_INET, SOCK_RAW | SOCK_NONBLOCK, IPPROTO_ICMP));

	struct sockaddr_in addr1;
	addr1.sin_family = AF_INET;
	addr1.sin_port = htons(9001);
	CHECK(inet_aton("127.0.0.1", &addr1.sin_addr));

	TEST_SUCC(bind(sk1, (struct sockaddr *)&addr1, sizeof(addr1)));

	TEST_ERRNO(bind(sk2, (struct sockaddr *)&addr1, sizeof(addr1)),
		   EADDRINUSE);

	TEST_SUCC(close(sk1));
	TEST_SUCC(close(sk2));
}
END_TEST()

FN_TEST(bind_different_protocol_same_addr)
{
	int sk1 = TEST_SUCC(
		socket(AF_INET, SOCK_RAW | SOCK_NONBLOCK, IPPROTO_ICMP));
	int sk2 = TEST_SUCC(
		socket(AF_INET, SOCK_RAW | SOCK_NONBLOCK, IPPROTO_IGMP));

	struct sockaddr_in addr1;
	addr1.sin_family = AF_INET;
	addr1.sin_port = htons(9002);
	CHECK(inet_aton("127.0.0.1", &addr1.sin_addr));

	TEST_SUCC(bind(sk1, (struct sockaddr *)&addr1, sizeof(addr1)));

	TEST_SUCC(bind(sk2, (struct sockaddr *)&addr1, sizeof(addr1)));

	TEST_SUCC(close(sk1));
	TEST_SUCC(close(sk2));
}
END_TEST()

FN_TEST(getsockname)
{
	struct sockaddr_in saddr;
	struct sockaddr *psaddr = (struct sockaddr *)&saddr;
	socklen_t addrlen = sizeof(saddr);

	saddr.sin_port = 0xbeef;
	TEST_RES(getsockname(sk_unbound, psaddr, &addrlen),
		 addrlen == sizeof(saddr) && saddr.sin_port == 0xbeef);

	saddr.sin_port = 0xbeef;
	addrlen = sizeof(saddr);
	TEST_RES(getsockname(sk_raw_icmp, psaddr, &addrlen),
		 addrlen == sizeof(saddr) && saddr.sin_port == TEST_ADDR);
}
END_TEST()

FN_TEST(getpeername)
{
	struct sockaddr_in saddr;
	struct sockaddr *psaddr = (struct sockaddr *)&saddr;
	socklen_t addrlen = sizeof(saddr);

	TEST_ERRNO(getpeername(sk_raw_icmp, psaddr, &addrlen), ENOTCONN);
}
END_TEST()

FN_TEST(recv_empty)
{
	char buf[1024];

	TEST_ERRNO(recv(sk_unbound, buf, sizeof(buf), 0), EAGAIN);
	TEST_ERRNO(recv(sk_raw_icmp, buf, sizeof(buf), 0), EAGAIN);
}
END_TEST()

FN_TEST(recvfrom_empty)
{
	char buf[1024];
	struct sockaddr_in saddr;
	socklen_t addrlen = sizeof(saddr);

	TEST_ERRNO(recvfrom(sk_raw_icmp, buf, sizeof(buf), 0,
			    (struct sockaddr *)&saddr, &addrlen),
		   EAGAIN);
}
END_TEST()

FN_TEST(send_no_dest)
{
	char buf[64] = { 0 };

	TEST_ERRNO(send(sk_raw_icmp, buf, sizeof(buf), 0), EDESTADDRREQ);
	TEST_ERRNO(write(sk_raw_icmp, buf, sizeof(buf)), EDESTADDRREQ);
}
END_TEST()

FN_TEST(poll)
{
	struct pollfd pfd = { .events = POLLIN | POLLOUT };

	pfd.fd = sk_unbound;
	TEST_RES(poll(&pfd, 1, 0),
		 (pfd.revents & (POLLIN | POLLOUT)) == POLLOUT);

	pfd.fd = sk_raw_icmp;
	TEST_RES(poll(&pfd, 1, 0),
		 (pfd.revents & (POLLIN | POLLOUT)) == POLLOUT);
}
END_TEST()

FN_TEST(listen_not_supported)
{
	TEST_ERRNO(listen(sk_raw_icmp, 2), EOPNOTSUPP);
}
END_TEST()

FN_TEST(accept_not_supported)
{
	struct sockaddr_in saddr;
	struct sockaddr *psaddr = (struct sockaddr *)&saddr;
	socklen_t addrlen = sizeof(saddr);

	TEST_ERRNO(accept(sk_raw_icmp, psaddr, &addrlen), EOPNOTSUPP);
}
END_TEST()

FN_TEST(connect_raw_icmp)
{
	struct sockaddr_in dst;
	dst.sin_family = AF_INET;
	dst.sin_port = 0;
	CHECK(inet_aton("127.0.0.1", &dst.sin_addr));

	TEST_SUCC(connect(sk_raw_icmp, (struct sockaddr *)&dst, sizeof(dst)));

	struct sockaddr_in peername;
	socklen_t peerlen = sizeof(peername);
	TEST_RES(getpeername(sk_raw_icmp, (struct sockaddr *)&peername,
			     &peerlen),
		 peerlen == sizeof(peername) &&
			 peername.sin_addr.s_addr == dst.sin_addr.s_addr);
}
END_TEST()

FN_TEST(close_sockets)
{
	TEST_SUCC(close(sk_raw_icmp));
	TEST_SUCC(close(sk_raw_udp));
	TEST_SUCC(close(sk_unbound));
}
END_TEST()
