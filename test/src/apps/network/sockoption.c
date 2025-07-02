// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <unistd.h>
#include <arpa/inet.h>
#include "../test.h"

int sk_unbound;
int sk_listen;
int sk_connected;
int sk_accepted;

struct sockaddr_in listen_addr;
#define LISTEN_PORT htons(0x1242)

FN_SETUP(general)
{
	sk_unbound = CHECK(socket(AF_INET, SOCK_STREAM, 0));

	listen_addr.sin_family = AF_INET;
	listen_addr.sin_port = LISTEN_PORT;
	CHECK(inet_aton("127.0.0.1", &listen_addr.sin_addr));

	sk_listen = CHECK(socket(AF_INET, SOCK_STREAM, 0));
	CHECK(bind(sk_listen, (struct sockaddr *)&listen_addr,
		   sizeof(listen_addr)));
	CHECK(listen(sk_listen, 3));

	sk_connected = CHECK(socket(AF_INET, SOCK_STREAM, 0));
	CHECK(connect(sk_connected, (struct sockaddr *)&listen_addr,
		      sizeof(listen_addr)));

	sk_accepted = CHECK(accept(sk_listen, NULL, NULL));
}
END_SETUP()

FN_TEST(invalid_socket_option)
{
	int res;
	socklen_t res_len = sizeof(res);

#define INVALID_LEVEL 99999
	TEST_ERRNO(getsockopt(sk_connected, INVALID_LEVEL, SO_SNDBUF, &res,
			      &res_len),
		   EOPNOTSUPP);
#define INVALID_SOCKET_OPTION 99999
	TEST_ERRNO(getsockopt(sk_connected, SOL_SOCKET, INVALID_SOCKET_OPTION,
			      &res, &res_len),
		   ENOPROTOOPT);
#define INVALID_TCP_OPTION 99999
	TEST_ERRNO(getsockopt(sk_connected, IPPROTO_TCP, INVALID_TCP_OPTION,
			      &res, &res_len),
		   ENOPROTOOPT);
#define INVALID_IP_OPTION 99999
	TEST_ERRNO(getsockopt(sk_connected, IPPROTO_IP, INVALID_IP_OPTION, &res,
			      &res_len),
		   ENOPROTOOPT);
}
END_TEST()

int refresh_connection()
{
	close(sk_connected);
	close(sk_accepted);

	sk_connected = CHECK(socket(AF_INET, SOCK_STREAM, 0));
	CHECK(connect(sk_connected, (struct sockaddr *)&listen_addr,
		      sizeof(listen_addr)));

	sk_accepted = CHECK(accept(sk_listen, NULL, NULL));

	return 0;
}

FN_TEST(buffer_size)
{
	int sendbuf;
	socklen_t sendbuf_len = sizeof(sendbuf);

	TEST_RES(getsockopt(sk_unbound, SOL_SOCKET, SO_SNDBUF, &sendbuf,
			    &sendbuf_len),
		 sendbuf_len == sizeof(sendbuf));
}
END_TEST()

FN_TEST(socket_error)
{
	int error;
	socklen_t error_len = sizeof(error);

	TEST_RES(getsockopt(sk_unbound, SOL_SOCKET, SO_ERROR, &error,
			    &error_len),
		 error_len == sizeof(error) && error == 0);
}
END_TEST()

FN_TEST(nagle)
{
	int option = 1;
	int nagle;
	socklen_t nagle_len = sizeof(nagle);

	// 1. Check default values
	refresh_connection();
	TEST_RES(getsockopt(sk_connected, IPPROTO_TCP, TCP_NODELAY, &nagle,
			    &nagle_len),
		 nagle == 0);
	TEST_RES(getsockopt(sk_accepted, IPPROTO_TCP, TCP_NODELAY, &nagle,
			    &nagle_len),
		 nagle == 0);

	// 2. Disable Nagle algorithm on unbound socket
	CHECK(setsockopt(sk_unbound, IPPROTO_TCP, TCP_NODELAY, &option,
			 sizeof(option)));
	TEST_RES(getsockopt(sk_unbound, IPPROTO_TCP, TCP_NODELAY, &nagle,
			    &nagle_len),
		 nagle == 1);

	// 3. Disable Nagle algorithm on connected socket
	CHECK(setsockopt(sk_connected, IPPROTO_TCP, TCP_NODELAY, &option,
			 sizeof(option)));
	TEST_RES(getsockopt(sk_connected, IPPROTO_TCP, TCP_NODELAY, &nagle,
			    &nagle_len),
		 nagle == 1);

	// 4. Disable Nagle algorithm on listening socket before connection
	CHECK(setsockopt(sk_listen, IPPROTO_TCP, TCP_NODELAY, &option,
			 sizeof(option)));
	TEST_RES(getsockopt(sk_listen, IPPROTO_TCP, TCP_NODELAY, &nagle,
			    &nagle_len),
		 nagle == 1);

	refresh_connection();
	TEST_RES(getsockopt(sk_connected, IPPROTO_TCP, TCP_NODELAY, &nagle,
			    &nagle_len),
		 nagle == 0);
	TEST_RES(getsockopt(sk_accepted, IPPROTO_TCP, TCP_NODELAY, &nagle,
			    &nagle_len),
		 nagle == 1);

	// 5. Disable Nagle algorithm on listening socket after connection
	option = 0;
	CHECK(setsockopt(sk_listen, IPPROTO_TCP, TCP_NODELAY, &option,
			 sizeof(option)));

	close(sk_connected);
	close(sk_accepted);

	sk_connected = CHECK(socket(AF_INET, SOCK_STREAM, 0));
	CHECK(connect(sk_connected, (struct sockaddr *)&listen_addr,
		      sizeof(listen_addr)));

	option = 1;
	CHECK(setsockopt(sk_listen, IPPROTO_TCP, TCP_NODELAY, &option,
			 sizeof(option)));

	sk_accepted = CHECK(accept(sk_listen, NULL, NULL));

	TEST_RES(getsockopt(sk_connected, IPPROTO_TCP, TCP_NODELAY, &nagle,
			    &nagle_len),
		 nagle == 0);
	TEST_RES(getsockopt(sk_accepted, IPPROTO_TCP, TCP_NODELAY, &nagle,
			    &nagle_len),
		 nagle == 0);
}
END_TEST()

FN_TEST(reuseaddr)
{
	int option = 1;
	CHECK(setsockopt(sk_unbound, SOL_SOCKET, SO_REUSEADDR, &option,
			 sizeof(option)));

	int reuseaddr;
	socklen_t reuseaddr_len = sizeof(reuseaddr);

	TEST_RES(getsockopt(sk_unbound, SOL_SOCKET, SO_REUSEADDR, &reuseaddr,
			    &reuseaddr_len),
		 reuseaddr == 1);
}
END_TEST()

FN_TEST(keepalive)
{
	int option = 1;
	int keepalive;
	socklen_t keepalive_len = sizeof(keepalive);

	// 1. Check default values
	refresh_connection();
	TEST_RES(getsockopt(sk_connected, SOL_SOCKET, SO_KEEPALIVE, &keepalive,
			    &keepalive_len),
		 keepalive == 0);
	TEST_RES(getsockopt(sk_accepted, SOL_SOCKET, SO_KEEPALIVE, &keepalive,
			    &keepalive_len),
		 keepalive == 0);

	// 2. Enable keepalive on unbound socket
	CHECK(setsockopt(sk_unbound, SOL_SOCKET, SO_KEEPALIVE, &option,
			 sizeof(option)));
	TEST_RES(getsockopt(sk_unbound, SOL_SOCKET, SO_KEEPALIVE, &keepalive,
			    &keepalive_len),
		 keepalive == 1);

	// 3. Enable keepalive on connected socket
	CHECK(setsockopt(sk_connected, SOL_SOCKET, SO_KEEPALIVE, &option,
			 sizeof(option)));
	TEST_RES(getsockopt(sk_connected, SOL_SOCKET, SO_KEEPALIVE, &keepalive,
			    &keepalive_len),
		 keepalive == 1);

	// 4. Enable keepalive on listening socket
	CHECK(setsockopt(sk_listen, SOL_SOCKET, SO_KEEPALIVE, &option,
			 sizeof(option)));
	TEST_RES(getsockopt(sk_listen, SOL_SOCKET, SO_KEEPALIVE, &keepalive,
			    &keepalive_len),
		 keepalive == 1);

	refresh_connection();
	TEST_RES(getsockopt(sk_connected, SOL_SOCKET, SO_KEEPALIVE, &keepalive,
			    &keepalive_len),
		 keepalive == 0);
	TEST_RES(getsockopt(sk_accepted, SOL_SOCKET, SO_KEEPALIVE, &keepalive,
			    &keepalive_len),
		 keepalive == 1);

	// 5. Setting keepalive after connection comes
	option = 0;
	CHECK(setsockopt(sk_listen, SOL_SOCKET, SO_KEEPALIVE, &option,
			 sizeof(option)));

	close(sk_connected);
	close(sk_accepted);

	sk_connected = CHECK(socket(AF_INET, SOCK_STREAM, 0));
	CHECK(connect(sk_connected, (struct sockaddr *)&listen_addr,
		      sizeof(listen_addr)));

	option = 1;
	CHECK(setsockopt(sk_listen, SOL_SOCKET, SO_KEEPALIVE, &option,
			 sizeof(option)));

	sk_accepted = CHECK(accept(sk_listen, NULL, NULL));

	TEST_RES(getsockopt(sk_connected, SOL_SOCKET, SO_KEEPALIVE, &keepalive,
			    &keepalive_len),
		 keepalive == 0);
	TEST_RES(getsockopt(sk_accepted, SOL_SOCKET, SO_KEEPALIVE, &keepalive,
			    &keepalive_len),
		 keepalive == 0);
}
END_TEST()

FN_TEST(keepidle)
{
	int keepidle;
	socklen_t keepidle_len = sizeof(keepidle);

	// 1. Check default values
	refresh_connection();
	TEST_RES(getsockopt(sk_connected, IPPROTO_TCP, TCP_KEEPIDLE, &keepidle,
			    &keepidle_len),
		 keepidle == 7200);
	TEST_RES(getsockopt(sk_accepted, IPPROTO_TCP, TCP_KEEPIDLE, &keepidle,
			    &keepidle_len),
		 keepidle == 7200);

	// 2. Set and Get value
	int seconds = 200;
	CHECK(setsockopt(sk_connected, IPPROTO_TCP, TCP_KEEPIDLE, &seconds,
			 sizeof(seconds)));
	TEST_RES(getsockopt(sk_connected, IPPROTO_TCP, TCP_KEEPIDLE, &keepidle,
			    &keepidle_len),
		 keepidle == 200);
}
END_TEST()

FN_TEST(ip_tos)
{
	int tos;
	socklen_t tos_len = sizeof(tos);

	// 1. Check default value
	TEST_RES(getsockopt(sk_unbound, IPPROTO_IP, IP_TOS, &tos, &tos_len),
		 tos == 0 && tos_len == 4);

	// 2. Set and get value
	tos = 0x10;
	CHECK(setsockopt(sk_unbound, IPPROTO_IP, IP_TOS, &tos, tos_len));
	tos = 0;
	TEST_RES(getsockopt(sk_unbound, IPPROTO_IP, IP_TOS, &tos, &tos_len),
		 tos == 0x10 && tos_len == 4);

	tos = 0x123;
	CHECK(setsockopt(sk_unbound, IPPROTO_IP, IP_TOS, &tos, tos_len));
	tos = 0;
	TEST_RES(getsockopt(sk_unbound, IPPROTO_IP, IP_TOS, &tos, &tos_len),
		 tos == 32 && tos_len == 4);

	tos = 0x1111;
	CHECK(setsockopt(sk_unbound, IPPROTO_IP, IP_TOS, &tos, tos_len));
	tos = 0;
	TEST_RES(getsockopt(sk_unbound, IPPROTO_IP, IP_TOS, &tos, &tos_len),
		 tos == 16 && tos_len == 4);
}
END_TEST()

FN_TEST(ip_ttl)
{
	int ttl;
	socklen_t ttl_len = sizeof(ttl);

	// 1. Check default value
	TEST_RES(getsockopt(sk_unbound, IPPROTO_IP, IP_TTL, &ttl, &ttl_len),
		 ttl == 64 && ttl_len == 4);

	// 2. Set and get value
	ttl = 0x0;
	TEST_ERRNO(setsockopt(sk_unbound, IPPROTO_IP, IP_TTL, &ttl, ttl_len),
		   EINVAL);

	ttl = 0x100;
	TEST_ERRNO(setsockopt(sk_unbound, IPPROTO_IP, IP_TTL, &ttl, ttl_len),
		   EINVAL);

	ttl = 0x10;
	CHECK(setsockopt(sk_unbound, IPPROTO_IP, IP_TTL, &ttl, ttl_len));

	ttl = 0;
	TEST_RES(getsockopt(sk_unbound, IPPROTO_IP, IP_TTL, &ttl, &ttl_len),
		 ttl == 0x10 && ttl_len == 4);

	ttl = -1;
	CHECK(setsockopt(sk_unbound, IPPROTO_IP, IP_TTL, &ttl, ttl_len));

	ttl = 0;
	TEST_RES(getsockopt(sk_unbound, IPPROTO_IP, IP_TTL, &ttl, &ttl_len),
		 ttl == 64 && ttl_len == 4);
}
END_TEST()

FN_TEST(ip_hdrincl)
{
	int hdrincl;
	socklen_t hdrincl_len = sizeof(hdrincl);

	// 1. Check default value
	TEST_RES(getsockopt(sk_unbound, IPPROTO_IP, IP_HDRINCL, &hdrincl,
			    &hdrincl_len),
		 hdrincl == 0 && hdrincl_len == 4);

	// 2. Set value
	hdrincl = 0x10;
	TEST_ERRNO(setsockopt(sk_unbound, IPPROTO_IP, IP_HDRINCL, &hdrincl,
			      hdrincl_len),
		   ENOPROTOOPT);
}
END_TEST()
