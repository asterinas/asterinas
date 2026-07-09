// SPDX-License-Identifier: MPL-2.0

#include <errno.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <unistd.h>
#include "../common/test.h"

// ============================================================
// IPV6_V6ONLY option tests
// ============================================================

FN_TEST(v6only_default_is_zero)
{
	int sk = TEST_SUCC(socket(AF_INET6, SOCK_STREAM, 0));

	int v6only;
	socklen_t len = sizeof(v6only);
	TEST_RES(getsockopt(sk, IPPROTO_IPV6, IPV6_V6ONLY, &v6only, &len),
		 v6only == 0);

	TEST_SUCC(close(sk));
}
END_TEST()

FN_TEST(v6only_toggle)
{
	int sk = TEST_SUCC(socket(AF_INET6, SOCK_STREAM, 0));

	int val = 0;
	TEST_SUCC(setsockopt(sk, IPPROTO_IPV6, IPV6_V6ONLY, &val, sizeof(val)));

	val = -1;
	socklen_t len = sizeof(val);
	TEST_RES(getsockopt(sk, IPPROTO_IPV6, IPV6_V6ONLY, &val, &len),
		 val == 0);

	val = 1;
	TEST_SUCC(setsockopt(sk, IPPROTO_IPV6, IPV6_V6ONLY, &val, sizeof(val)));

	val = 0;
	len = sizeof(val);
	TEST_RES(getsockopt(sk, IPPROTO_IPV6, IPV6_V6ONLY, &val, &len),
		 val == 1);

	TEST_SUCC(close(sk));
}
END_TEST()

FN_TEST(v6only_rejected_on_af_inet)
{
	int sk = TEST_SUCC(socket(AF_INET, SOCK_STREAM, 0));

	int val;
	socklen_t len = sizeof(val);
	TEST_ERRNO(getsockopt(sk, IPPROTO_IPV6, IPV6_V6ONLY, &val, &len),
		   ENOPROTOOPT);

	val = 0;
	TEST_ERRNO(setsockopt(sk, IPPROTO_IPV6, IPV6_V6ONLY, &val, sizeof(val)),
		   ENOPROTOOPT);

	TEST_SUCC(close(sk));
}
END_TEST()

// ============================================================
// v6only=1: reject IPv4 and IPv4-mapped IPv6 addresses
// ============================================================

FN_TEST(v6only_reject_mapped_bind)
{
	int sk = TEST_SUCC(socket(AF_INET6, SOCK_STREAM, 0));
	int v6only = 1;
	TEST_SUCC(setsockopt(sk, IPPROTO_IPV6, IPV6_V6ONLY, &v6only,
			     sizeof(v6only)));

	// ::ffff:127.0.0.1
	struct sockaddr_in6 addr = { 0 };
	addr.sin6_family = AF_INET6;
	addr.sin6_port = htons(8080);
	addr.sin6_addr.s6_addr[10] = 0xff;
	addr.sin6_addr.s6_addr[11] = 0xff;
	addr.sin6_addr.s6_addr[12] = 127;
	addr.sin6_addr.s6_addr[13] = 0;
	addr.sin6_addr.s6_addr[14] = 0;
	addr.sin6_addr.s6_addr[15] = 1;

	TEST_ERRNO(bind(sk, (struct sockaddr *)&addr, sizeof(addr)),
		   EAFNOSUPPORT);

	TEST_SUCC(close(sk));
}
END_TEST()

FN_TEST(v6only_reject_ipv4_connect)
{
	int sk = TEST_SUCC(socket(AF_INET6, SOCK_STREAM, 0));
	int v6only = 1;
	TEST_SUCC(setsockopt(sk, IPPROTO_IPV6, IPV6_V6ONLY, &v6only,
			     sizeof(v6only)));

	struct sockaddr_in addr = { 0 };
	addr.sin_family = AF_INET;
	addr.sin_port = htons(8080);
	addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);

	TEST_ERRNO(connect(sk, (struct sockaddr *)&addr, sizeof(addr)),
		   EAFNOSUPPORT);

	TEST_SUCC(close(sk));
}
END_TEST()

// ============================================================
// v6only=0 dual-stack: bind / listen / connect / accept
// ============================================================

static int make_dualstack_listener(struct sockaddr_in6 *addr,
				   socklen_t *addrlen)
{
	int sk = CHECK(socket(AF_INET6, SOCK_STREAM, 0));
	int v6only = 0;
	CHECK(setsockopt(sk, IPPROTO_IPV6, IPV6_V6ONLY, &v6only,
			 sizeof(v6only)));

	struct sockaddr_in6 bind_addr = { 0 };
	bind_addr.sin6_family = AF_INET6;
	bind_addr.sin6_port = htons(0);
	bind_addr.sin6_addr = in6addr_loopback;
	CHECK(bind(sk, (struct sockaddr *)&bind_addr, sizeof(bind_addr)));
	CHECK(listen(sk, 1));

	if (addr)
		CHECK(getsockname(sk, (struct sockaddr *)addr, addrlen));
	return sk;
}

FN_TEST(dualstack_bind_getsockname)
{
	struct sockaddr_in6 addr = { 0 };
	socklen_t addrlen = sizeof(addr);
	int sk = make_dualstack_listener(&addr, &addrlen);

	TEST_RES(addr.sin6_family, addr.sin6_family == AF_INET6);
	TEST_RES(ntohs(addr.sin6_port), ntohs(addr.sin6_port) != 0);

	TEST_SUCC(close(sk));
}
END_TEST()

FN_TEST(dualstack_connect_ipv4_peer)
{
	struct sockaddr_in6 listen_addr = { 0 };
	socklen_t listen_len = sizeof(listen_addr);
	int listener = make_dualstack_listener(&listen_addr, &listen_len);

	int sk = TEST_SUCC(socket(AF_INET6, SOCK_STREAM, 0));
	int v6only = 0;
	TEST_SUCC(setsockopt(sk, IPPROTO_IPV6, IPV6_V6ONLY, &v6only,
			     sizeof(v6only)));

	// Connect with bare IPv4 to the dual-stack listener
	struct sockaddr_in v4_addr = { 0 };
	v4_addr.sin_family = AF_INET;
	v4_addr.sin_port = listen_addr.sin6_port;
	v4_addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
	TEST_SUCC(connect(sk, (struct sockaddr *)&v4_addr, sizeof(v4_addr)));

	// getpeername: must present as IPv4-mapped IPv6 per RFC 4038
	struct sockaddr_in6 peer = { 0 };
	socklen_t peer_len = sizeof(peer);
	TEST_SUCC(getpeername(sk, (struct sockaddr *)&peer, &peer_len));
	TEST_RES(peer.sin6_family, peer.sin6_family == AF_INET6);
	TEST_RES(ntohs(peer.sin6_port),
		 ntohs(peer.sin6_port) == ntohs(listen_addr.sin6_port));
	// Check ::ffff:127.0.0.1 prefix
	TEST_RES(peer.sin6_addr.s6_addr[10],
		 peer.sin6_addr.s6_addr[10] == 0xff);
	TEST_RES(peer.sin6_addr.s6_addr[11],
		 peer.sin6_addr.s6_addr[11] == 0xff);

	// getsockname: also IPv6 form
	struct sockaddr_in6 local = { 0 };
	socklen_t local_len = sizeof(local);
	TEST_SUCC(getsockname(sk, (struct sockaddr *)&local, &local_len));
	TEST_RES(local.sin6_family, local.sin6_family == AF_INET6);

	TEST_SUCC(close(sk));

	// Accept from listener side to clean up
	int accepted = TEST_SUCC(accept(listener, NULL, NULL));
	TEST_SUCC(close(accepted));
	TEST_SUCC(close(listener));
}
END_TEST()

FN_TEST(dualstack_broadcast_connect_no_so_broadcast)
{
	int sk = TEST_SUCC(socket(AF_INET6, SOCK_STREAM, 0));
	int v6only = 0;
	TEST_SUCC(setsockopt(sk, IPPROTO_IPV6, IPV6_V6ONLY, &v6only,
			     sizeof(v6only)));

	struct sockaddr_in addr = { 0 };
	addr.sin_family = AF_INET;
	addr.sin_port = htons(12345);
	addr.sin_addr.s_addr = htonl(INADDR_BROADCAST);

	TEST_ERRNO(connect(sk, (struct sockaddr *)&addr, sizeof(addr)), EACCES);

	TEST_SUCC(close(sk));
}
END_TEST()
