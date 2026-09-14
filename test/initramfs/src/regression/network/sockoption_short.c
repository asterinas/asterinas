// SPDX-License-Identifier: MPL-2.0

#include <netinet/in.h>
#include <netinet/tcp.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <unistd.h>
#include "../common/test.h"

int sk_tcp;

#define TEST_GETSOCKOPT(level, option, expected, optlen)                       \
	do {                                                                   \
		unsigned char test_value_[sizeof(expected)];                   \
		unsigned char unchanged_[sizeof(expected)];                    \
		socklen_t requested_len_ = (optlen);                           \
		socklen_t actual_len_ = requested_len_;                        \
		memset(test_value_, 0xa5, sizeof(test_value_));                \
		memset(unchanged_, 0xa5, sizeof(unchanged_));                  \
		TEST_RES(getsockopt(sk_tcp, level, option, test_value_,        \
				    &actual_len_),                             \
			 actual_len_ == requested_len_ &&                      \
				 memcmp(test_value_, &(expected),              \
					actual_len_) == 0 &&                   \
				 memcmp(test_value_ + actual_len_,             \
					unchanged_ + actual_len_,              \
					sizeof(expected) - actual_len_) == 0); \
	} while (0)

FN_SETUP(general)
{
	sk_tcp = CHECK(socket(AF_INET, SOCK_STREAM, 0));
}
END_SETUP()

FN_TEST(negative_optlen)
{
	int value;
	socklen_t len;

	len = (socklen_t)-1;
	TEST_ERRNO(getsockopt(sk_tcp, SOL_SOCKET, SO_KEEPALIVE, &value, &len),
		   EINVAL);
	len = (socklen_t)-1;
	TEST_ERRNO(getsockopt(sk_tcp, IPPROTO_TCP, TCP_KEEPCNT, &value, &len),
		   EINVAL);
	len = (socklen_t)-1;
	TEST_ERRNO(getsockopt(sk_tcp, IPPROTO_IP, IP_TOS, &value, &len),
		   EINVAL);

	TEST_ERRNO(setsockopt(sk_tcp, SOL_SOCKET, SO_KEEPALIVE, &value,
			      (socklen_t)-1),
		   EINVAL);
	TEST_ERRNO(setsockopt(sk_tcp, IPPROTO_TCP, TCP_KEEPCNT, &value,
			      (socklen_t)-1),
		   EINVAL);
	TEST_ERRNO(setsockopt(sk_tcp, IPPROTO_IP, IP_TOS, &value,
			      (socklen_t)-1),
		   EINVAL);
}
END_TEST()

FN_TEST(short_optlen)
{
	int keepalive = 1;

	TEST_SUCC(setsockopt(sk_tcp, SOL_SOCKET, SO_KEEPALIVE, &keepalive,
			     sizeof(keepalive)));
	TEST_ERRNO(setsockopt(sk_tcp, SOL_SOCKET, SO_KEEPALIVE, &keepalive,
			      sizeof(keepalive) - 1),
		   EINVAL);

	TEST_GETSOCKOPT(SOL_SOCKET, SO_KEEPALIVE, keepalive, sizeof(keepalive));
	TEST_GETSOCKOPT(SOL_SOCKET, SO_KEEPALIVE, keepalive,
			sizeof(keepalive) - 1);
	TEST_GETSOCKOPT(SOL_SOCKET, SO_KEEPALIVE, keepalive, 0);

	int keepcnt = 5;

	TEST_SUCC(setsockopt(sk_tcp, IPPROTO_TCP, TCP_KEEPCNT, &keepcnt,
			     sizeof(keepcnt)));
	TEST_ERRNO(setsockopt(sk_tcp, IPPROTO_TCP, TCP_KEEPCNT, &keepcnt, 1),
		   EINVAL);

	TEST_GETSOCKOPT(IPPROTO_TCP, TCP_KEEPCNT, keepcnt, sizeof(keepcnt));
	TEST_GETSOCKOPT(IPPROTO_TCP, TCP_KEEPCNT, keepcnt, 1);
	TEST_GETSOCKOPT(IPPROTO_TCP, TCP_KEEPCNT, keepcnt, 0);
}
END_TEST()

FN_TEST(short_ip_optlen)
{
	unsigned char value;
	int expected;

	value = 0x10;
	TEST_SUCC(
		setsockopt(sk_tcp, IPPROTO_IP, IP_TOS, &value, sizeof(value)));
	expected = value;
	TEST_GETSOCKOPT(IPPROTO_IP, IP_TOS, expected, sizeof(expected));

	TEST_SUCC(setsockopt(sk_tcp, IPPROTO_IP, IP_TOS, NULL, 0));

	value = 42;
	TEST_SUCC(
		setsockopt(sk_tcp, IPPROTO_IP, IP_TTL, &value, sizeof(value)));
	expected = value;
	TEST_GETSOCKOPT(IPPROTO_IP, IP_TTL, expected, sizeof(expected));

	// Linux treats an empty value as zero, while IP_TTL only accepts -1 or a
	// value in the range 1..=255, so an empty value results in EINVAL. See
	// https://elixir.bootlin.com/linux/v7.1/source/net/ipv4/ip_sockglue.c#L939
	// and https://elixir.bootlin.com/linux/v7.1/source/net/ipv4/ip_sockglue.c#L1026-L1030.
	TEST_ERRNO(setsockopt(sk_tcp, IPPROTO_IP, IP_TTL, NULL, 0), EINVAL);

	value = 1;
	TEST_SUCC(setsockopt(sk_tcp, IPPROTO_IP, IP_RECVERR, &value,
			     sizeof(value)));
	expected = value;
	TEST_GETSOCKOPT(IPPROTO_IP, IP_RECVERR, expected, sizeof(expected));

	TEST_SUCC(setsockopt(sk_tcp, IPPROTO_IP, IP_RECVERR, NULL, 0));
}
END_TEST()

FN_TEST(short_struct_optlen)
{
	socklen_t len;

	struct linger linger = { .l_onoff = 1, .l_linger = 0x12345678 };

	TEST_SUCC(setsockopt(sk_tcp, SOL_SOCKET, SO_LINGER, &linger,
			     sizeof(linger)));
	TEST_ERRNO(setsockopt(sk_tcp, SOL_SOCKET, SO_LINGER, &linger,
			      sizeof(linger.l_onoff)),
		   EINVAL);
	TEST_ERRNO(setsockopt(sk_tcp, SOL_SOCKET, SO_LINGER, NULL, 0), EINVAL);
	TEST_ERRNO(setsockopt(sk_tcp, SOL_SOCKET, SO_LINGER, &linger, 1),
		   EINVAL);

	TEST_GETSOCKOPT(SOL_SOCKET, SO_LINGER, linger, sizeof(linger));
	TEST_GETSOCKOPT(SOL_SOCKET, SO_LINGER, linger, sizeof(linger.l_onoff));
	TEST_GETSOCKOPT(SOL_SOCKET, SO_LINGER, linger, 1);
	len = 0;
	TEST_SUCC(getsockopt(sk_tcp, SOL_SOCKET, SO_LINGER, NULL, &len));

	struct timeval timeout = { .tv_sec = 3 };

	TEST_SUCC(setsockopt(sk_tcp, SOL_SOCKET, SO_RCVTIMEO, &timeout,
			     sizeof(timeout)));
	TEST_ERRNO(setsockopt(sk_tcp, SOL_SOCKET, SO_RCVTIMEO, &timeout,
			      sizeof(timeout.tv_sec)),
		   EINVAL);
	TEST_ERRNO(setsockopt(sk_tcp, SOL_SOCKET, SO_RCVTIMEO, NULL, 0),
		   EINVAL);
	TEST_ERRNO(setsockopt(sk_tcp, SOL_SOCKET, SO_RCVTIMEO, &timeout, 1),
		   EINVAL);

	TEST_GETSOCKOPT(SOL_SOCKET, SO_RCVTIMEO, timeout, sizeof(timeout));
	TEST_GETSOCKOPT(SOL_SOCKET, SO_RCVTIMEO, timeout,
			sizeof(timeout.tv_sec));
	TEST_GETSOCKOPT(SOL_SOCKET, SO_RCVTIMEO, timeout, 1);
	len = 0;
	TEST_SUCC(getsockopt(sk_tcp, SOL_SOCKET, SO_RCVTIMEO, NULL, &len));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(sk_tcp));
}
END_SETUP()
