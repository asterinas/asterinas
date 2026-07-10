// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <unistd.h>
#include <arpa/inet.h>
#include "../common/test.h"

int sk_unbound;
int sk_listen;
int sk_connected;
int sk_accepted;
int sk_udp;

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

	sk_udp = CHECK(socket(AF_INET, SOCK_DGRAM, 0));
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

FN_TEST(null_optlen)
{
	int val;
	TEST_ERRNO(getsockopt(sk_connected, SOL_SOCKET, SO_KEEPALIVE, &val,
			      NULL),
		   EFAULT);
}
END_TEST()

FN_TEST(null_optval)
{
	socklen_t len = sizeof(int);
	TEST_ERRNO(setsockopt(sk_connected, SOL_SOCKET, SO_KEEPALIVE, NULL,
			      sizeof(int)),
		   EFAULT);
	TEST_ERRNO(getsockopt(sk_connected, SOL_SOCKET, SO_KEEPALIVE, NULL,
			      &len),
		   EFAULT);
}
END_TEST()

FN_TEST(short_optlen)
{
	int expected = 1;
	unsigned char value[sizeof(expected)];
	socklen_t len;

	TEST_SUCC(setsockopt(sk_connected, SOL_SOCKET, SO_KEEPALIVE, &expected,
			     sizeof(expected)));

	memset(value, 0xa5, sizeof(value));
	len = 2;
	TEST_RES(getsockopt(sk_connected, SOL_SOCKET, SO_KEEPALIVE, value,
			    &len),
		 _ret == 0 && len == 2);
	TEST_RES(memcmp(value, &expected, len), _ret == 0);
	TEST_RES(value[2], _ret == 0xa5);
	TEST_RES(value[3], _ret == 0xa5);

	memset(value, 0xa5, sizeof(value));
	len = 0;
	TEST_RES(getsockopt(sk_connected, SOL_SOCKET, SO_KEEPALIVE, value,
			    &len),
		 _ret == 0 && len == 0);
	TEST_RES(value[0], _ret == 0xa5);
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

FN_TEST(socket_type)
{
	int type = -1;
	socklen_t type_len = sizeof(type);

	TEST_ERRNO(setsockopt(sk_unbound, SOL_SOCKET, SO_TYPE, &type, type_len),
		   ENOPROTOOPT);

	TEST_RES(getsockopt(sk_unbound, SOL_SOCKET, SO_TYPE, &type, &type_len),
		 type == SOCK_STREAM && type_len == sizeof(type));

	TEST_RES(getsockopt(sk_udp, SOL_SOCKET, SO_TYPE, &type, &type_len),
		 type == SOCK_DGRAM && type_len == sizeof(type));
}
END_TEST()

FN_TEST(socket_timeout)
{
	struct timeval timeout;
	socklen_t timeout_len;
	char buf;

	timeout_len = sizeof(timeout);
	TEST_RES(getsockopt(sk_connected, SOL_SOCKET, SO_RCVTIMEO, &timeout,
			    &timeout_len),
		 timeout.tv_sec == 0 && timeout.tv_usec == 0 &&
			 timeout_len == sizeof(timeout));

	timeout = (struct timeval){ .tv_sec = 0, .tv_usec = 200000 };
	TEST_SUCC(setsockopt(sk_connected, SOL_SOCKET, SO_RCVTIMEO, &timeout,
			     sizeof(timeout)));

	timeout_len = sizeof(timeout);
	TEST_RES(getsockopt(sk_connected, SOL_SOCKET, SO_RCVTIMEO, &timeout,
			    &timeout_len),
		 timeout.tv_sec == 0 && timeout.tv_usec == 200000 &&
			 timeout_len == sizeof(timeout));

	timeout = (struct timeval){ .tv_sec = 0, .tv_usec = 100000 };
	TEST_SUCC(setsockopt(sk_connected, SOL_SOCKET, SO_SNDTIMEO, &timeout,
			     sizeof(timeout)));

	timeout_len = sizeof(timeout);
	TEST_RES(getsockopt(sk_connected, SOL_SOCKET, SO_SNDTIMEO, &timeout,
			    &timeout_len),
		 timeout.tv_sec == 0 && timeout.tv_usec == 100000 &&
			 timeout_len == sizeof(timeout));

	TEST_SUCC(close(sk_connected));
	TEST_SUCC(close(sk_accepted));

	timeout = (struct timeval){ .tv_sec = 0, .tv_usec = 500000 };
	TEST_SUCC(setsockopt(sk_listen, SOL_SOCKET, SO_RCVTIMEO, &timeout,
			     sizeof(timeout)));
	timeout = (struct timeval){ .tv_sec = 0, .tv_usec = 600000 };
	TEST_SUCC(setsockopt(sk_listen, SOL_SOCKET, SO_SNDTIMEO, &timeout,
			     sizeof(timeout)));

	sk_connected = TEST_SUCC(socket(AF_INET, SOCK_STREAM, 0));
	TEST_SUCC(connect(sk_connected, (struct sockaddr *)&listen_addr,
			  sizeof(listen_addr)));

	timeout = (struct timeval){ .tv_sec = 0, .tv_usec = 700000 };
	TEST_SUCC(setsockopt(sk_listen, SOL_SOCKET, SO_RCVTIMEO, &timeout,
			     sizeof(timeout)));
	timeout = (struct timeval){ .tv_sec = 0, .tv_usec = 800000 };
	TEST_SUCC(setsockopt(sk_listen, SOL_SOCKET, SO_SNDTIMEO, &timeout,
			     sizeof(timeout)));

	sk_accepted = TEST_SUCC(accept(sk_listen, NULL, NULL));

	timeout_len = sizeof(timeout);
	TEST_RES(getsockopt(sk_accepted, SOL_SOCKET, SO_RCVTIMEO, &timeout,
			    &timeout_len),
		 timeout.tv_sec == 0 && timeout.tv_usec == 700000 &&
			 timeout_len == sizeof(timeout));

	timeout_len = sizeof(timeout);
	TEST_RES(getsockopt(sk_accepted, SOL_SOCKET, SO_SNDTIMEO, &timeout,
			    &timeout_len),
		 timeout.tv_sec == 0 && timeout.tv_usec == 800000 &&
			 timeout_len == sizeof(timeout));

	timeout = (struct timeval){ .tv_sec = -1, .tv_usec = 0 };
	TEST_SUCC(setsockopt(sk_connected, SOL_SOCKET, SO_RCVTIMEO, &timeout,
			     sizeof(timeout)));

	timeout_len = sizeof(timeout);
	TEST_RES(getsockopt(sk_connected, SOL_SOCKET, SO_RCVTIMEO, &timeout,
			    &timeout_len),
		 timeout.tv_sec == 0 && timeout.tv_usec == 0 &&
			 timeout_len == sizeof(timeout));

	timeout = (struct timeval){ .tv_sec = 0, .tv_usec = -1 };
	TEST_ERRNO(setsockopt(sk_connected, SOL_SOCKET, SO_RCVTIMEO, &timeout,
			      sizeof(timeout)),
		   EDOM);

	timeout = (struct timeval){ .tv_sec = 0, .tv_usec = 1000000 };
	TEST_ERRNO(setsockopt(sk_connected, SOL_SOCKET, SO_SNDTIMEO, &timeout,
			      sizeof(timeout)),
		   EDOM);

	timeout = (struct timeval){ .tv_sec = 0, .tv_usec = 200000 };
	TEST_SUCC(setsockopt(sk_connected, SOL_SOCKET, SO_RCVTIMEO, &timeout,
			     sizeof(timeout)));
	TEST_ERRNO(recv(sk_connected, &buf, sizeof(buf), 0), EAGAIN);

	timeout = (struct timeval){ .tv_sec = 0, .tv_usec = 0 };
	TEST_SUCC(setsockopt(sk_connected, SOL_SOCKET, SO_RCVTIMEO, &timeout,
			     sizeof(timeout)));
	TEST_SUCC(setsockopt(sk_connected, SOL_SOCKET, SO_SNDTIMEO, &timeout,
			     sizeof(timeout)));
	TEST_SUCC(setsockopt(sk_listen, SOL_SOCKET, SO_RCVTIMEO, &timeout,
			     sizeof(timeout)));
	TEST_SUCC(setsockopt(sk_listen, SOL_SOCKET, SO_SNDTIMEO, &timeout,
			     sizeof(timeout)));
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
	TEST_SUCC(setsockopt(sk_unbound, IPPROTO_TCP, TCP_NODELAY, &option,
			     sizeof(option)));
	TEST_RES(getsockopt(sk_unbound, IPPROTO_TCP, TCP_NODELAY, &nagle,
			    &nagle_len),
		 nagle == 1);

	// 3. Disable Nagle algorithm on connected socket
	TEST_SUCC(setsockopt(sk_connected, IPPROTO_TCP, TCP_NODELAY, &option,
			     sizeof(option)));
	TEST_RES(getsockopt(sk_connected, IPPROTO_TCP, TCP_NODELAY, &nagle,
			    &nagle_len),
		 nagle == 1);

	// 4. Disable Nagle algorithm on listening socket before connection
	TEST_SUCC(setsockopt(sk_listen, IPPROTO_TCP, TCP_NODELAY, &option,
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
	TEST_SUCC(setsockopt(sk_listen, IPPROTO_TCP, TCP_NODELAY, &option,
			     sizeof(option)));

	close(sk_connected);
	close(sk_accepted);

	sk_connected = TEST_SUCC(socket(AF_INET, SOCK_STREAM, 0));
	TEST_SUCC(connect(sk_connected, (struct sockaddr *)&listen_addr,
			  sizeof(listen_addr)));

	option = 1;
	TEST_SUCC(setsockopt(sk_listen, IPPROTO_TCP, TCP_NODELAY, &option,
			     sizeof(option)));

	sk_accepted = TEST_SUCC(accept(sk_listen, NULL, NULL));

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
	TEST_SUCC(setsockopt(sk_unbound, SOL_SOCKET, SO_REUSEADDR, &option,
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
	TEST_SUCC(setsockopt(sk_unbound, SOL_SOCKET, SO_KEEPALIVE, &option,
			     sizeof(option)));
	TEST_RES(getsockopt(sk_unbound, SOL_SOCKET, SO_KEEPALIVE, &keepalive,
			    &keepalive_len),
		 keepalive == 1);

	// 3. Enable keepalive on connected socket
	TEST_SUCC(setsockopt(sk_connected, SOL_SOCKET, SO_KEEPALIVE, &option,
			     sizeof(option)));
	TEST_RES(getsockopt(sk_connected, SOL_SOCKET, SO_KEEPALIVE, &keepalive,
			    &keepalive_len),
		 keepalive == 1);

	// 4. Enable keepalive on listening socket
	TEST_SUCC(setsockopt(sk_listen, SOL_SOCKET, SO_KEEPALIVE, &option,
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
	TEST_SUCC(setsockopt(sk_listen, SOL_SOCKET, SO_KEEPALIVE, &option,
			     sizeof(option)));

	close(sk_connected);
	close(sk_accepted);

	sk_connected = TEST_SUCC(socket(AF_INET, SOCK_STREAM, 0));
	TEST_SUCC(connect(sk_connected, (struct sockaddr *)&listen_addr,
			  sizeof(listen_addr)));

	option = 1;
	TEST_SUCC(setsockopt(sk_listen, SOL_SOCKET, SO_KEEPALIVE, &option,
			     sizeof(option)));

	sk_accepted = TEST_SUCC(accept(sk_listen, NULL, NULL));

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
	TEST_SUCC(setsockopt(sk_connected, IPPROTO_TCP, TCP_KEEPIDLE, &seconds,
			     sizeof(seconds)));
	TEST_RES(getsockopt(sk_connected, IPPROTO_TCP, TCP_KEEPIDLE, &keepidle,
			    &keepidle_len),
		 keepidle == 200);
}
END_TEST()

FN_TEST(keepintvl)
{
	int keepintvl;
	socklen_t keepintvl_len = sizeof(keepintvl);

	// 1. Check default values
	refresh_connection();
	TEST_RES(getsockopt(sk_connected, IPPROTO_TCP, TCP_KEEPINTVL,
			    &keepintvl, &keepintvl_len),
		 keepintvl == 75);
	TEST_RES(getsockopt(sk_accepted, IPPROTO_TCP, TCP_KEEPINTVL, &keepintvl,
			    &keepintvl_len),
		 keepintvl == 75);

	// 2. Set and get value
	int seconds = 30;
	TEST_SUCC(setsockopt(sk_connected, IPPROTO_TCP, TCP_KEEPINTVL, &seconds,
			     sizeof(seconds)));
	TEST_RES(getsockopt(sk_connected, IPPROTO_TCP, TCP_KEEPINTVL,
			    &keepintvl, &keepintvl_len),
		 keepintvl == 30);

	// 3. Inherit the value from the listening socket
	int enabled = 1;
	TEST_SUCC(setsockopt(sk_listen, IPPROTO_TCP, TCP_KEEPINTVL, &seconds,
			     sizeof(seconds)));
	TEST_SUCC(setsockopt(sk_listen, SOL_SOCKET, SO_KEEPALIVE, &enabled,
			     sizeof(enabled)));
	refresh_connection();
	TEST_RES(getsockopt(sk_accepted, IPPROTO_TCP, TCP_KEEPINTVL, &keepintvl,
			    &keepintvl_len),
		 keepintvl == 30);

	// 4. Inherit the value while keepalive is disabled
	enabled = 0;
	seconds = 50;
	TEST_SUCC(setsockopt(sk_listen, IPPROTO_TCP, TCP_KEEPINTVL, &seconds,
			     sizeof(seconds)));
	TEST_SUCC(setsockopt(sk_listen, SOL_SOCKET, SO_KEEPALIVE, &enabled,
			     sizeof(enabled)));
	refresh_connection();
	TEST_RES(getsockopt(sk_accepted, IPPROTO_TCP, TCP_KEEPINTVL, &keepintvl,
			    &keepintvl_len),
		 keepintvl == 50);
}
END_TEST()

FN_TEST(keepcnt)
{
	int keepcnt;
	socklen_t keepcnt_len = sizeof(keepcnt);

	// 1. Check default value
	refresh_connection();
	TEST_RES(getsockopt(sk_connected, IPPROTO_TCP, TCP_KEEPCNT, &keepcnt,
			    &keepcnt_len),
		 keepcnt == 9);

	// 2. Set and get value
	int count = 5;
	TEST_SUCC(setsockopt(sk_connected, IPPROTO_TCP, TCP_KEEPCNT, &count,
			     sizeof(count)));
	TEST_RES(getsockopt(sk_connected, IPPROTO_TCP, TCP_KEEPCNT, &keepcnt,
			    &keepcnt_len),
		 keepcnt == 5);
}
END_TEST()

FN_TEST(ip_tos)
{
	int tos;
	socklen_t tos_len = sizeof(tos);

	// 1. Check default value
	TEST_RES(getsockopt(sk_unbound, IPPROTO_IP, IP_TOS, &tos, &tos_len),
		 tos == 0 && tos_len == 4);
	TEST_RES(getsockopt(sk_udp, IPPROTO_IP, IP_TOS, &tos, &tos_len),
		 tos == 0 && tos_len == 4);

	// 2. Set and get value
	tos = 0x10;
	TEST_SUCC(setsockopt(sk_unbound, IPPROTO_IP, IP_TOS, &tos, tos_len));
	TEST_SUCC(setsockopt(sk_udp, IPPROTO_IP, IP_TOS, &tos, tos_len));
	tos = 0;
	TEST_RES(getsockopt(sk_unbound, IPPROTO_IP, IP_TOS, &tos, &tos_len),
		 tos == 0x10 && tos_len == 4);
	TEST_RES(getsockopt(sk_udp, IPPROTO_IP, IP_TOS, &tos, &tos_len),
		 tos == 0x10 && tos_len == 4);

	tos = 0x123;
	TEST_SUCC(setsockopt(sk_unbound, IPPROTO_IP, IP_TOS, &tos, tos_len));
	tos = 0;
	TEST_RES(getsockopt(sk_unbound, IPPROTO_IP, IP_TOS, &tos, &tos_len),
		 tos == 32 && tos_len == 4);

	tos = 0x1111;
	TEST_SUCC(setsockopt(sk_unbound, IPPROTO_IP, IP_TOS, &tos, tos_len));
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
	TEST_RES(getsockopt(sk_udp, IPPROTO_IP, IP_TTL, &ttl, &ttl_len),
		 ttl == 64 && ttl_len == 4);

	// 2. Set and get value
	ttl = 0x0;
	TEST_ERRNO(setsockopt(sk_unbound, IPPROTO_IP, IP_TTL, &ttl, ttl_len),
		   EINVAL);

	ttl = 0x100;
	TEST_ERRNO(setsockopt(sk_unbound, IPPROTO_IP, IP_TTL, &ttl, ttl_len),
		   EINVAL);

	ttl = 0x10;
	TEST_SUCC(setsockopt(sk_unbound, IPPROTO_IP, IP_TTL, &ttl, ttl_len));
	TEST_SUCC(setsockopt(sk_udp, IPPROTO_IP, IP_TTL, &ttl, ttl_len));

	ttl = 0;
	TEST_RES(getsockopt(sk_unbound, IPPROTO_IP, IP_TTL, &ttl, &ttl_len),
		 ttl == 0x10 && ttl_len == 4);
	TEST_RES(getsockopt(sk_udp, IPPROTO_IP, IP_TTL, &ttl, &ttl_len),
		 ttl == 0x10 && ttl_len == 4);

	ttl = -1;
	TEST_SUCC(setsockopt(sk_unbound, IPPROTO_IP, IP_TTL, &ttl, ttl_len));

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
	TEST_RES(getsockopt(sk_udp, IPPROTO_IP, IP_HDRINCL, &hdrincl,
			    &hdrincl_len),
		 hdrincl == 0 && hdrincl_len == 4);

	// 2. Set and get value
	hdrincl = 0x10;
	TEST_ERRNO(setsockopt(sk_unbound, IPPROTO_IP, IP_HDRINCL, &hdrincl,
			      hdrincl_len),
		   ENOPROTOOPT);
	TEST_ERRNO(setsockopt(sk_udp, IPPROTO_IP, IP_HDRINCL, &hdrincl,
			      hdrincl_len),
		   ENOPROTOOPT);
}
END_TEST()

FN_TEST(ip_recverr)
{
	int recverr;
	socklen_t recverr_len = sizeof(recverr);

	// 1. Check default value
	TEST_RES(getsockopt(sk_unbound, IPPROTO_IP, IP_RECVERR, &recverr,
			    &recverr_len),
		 recverr == 0 && recverr_len == 4);
	TEST_RES(getsockopt(sk_udp, IPPROTO_IP, IP_RECVERR, &recverr,
			    &recverr_len),
		 recverr == 0 && recverr_len == 4);

	// 2. Set and get value
	recverr = 100;
	TEST_SUCC(setsockopt(sk_unbound, IPPROTO_IP, IP_RECVERR, &recverr,
			     recverr_len));
	TEST_RES(getsockopt(sk_unbound, IPPROTO_IP, IP_RECVERR, &recverr,
			    &recverr_len),
		 recverr == 1 && recverr_len == 4);
	recverr = -1;
	TEST_SUCC(setsockopt(sk_udp, IPPROTO_IP, IP_RECVERR, &recverr,
			     recverr_len));
	TEST_RES(getsockopt(sk_udp, IPPROTO_IP, IP_RECVERR, &recverr,
			    &recverr_len),
		 recverr == 1 && recverr_len == 4);
}
END_TEST()
