// SPDX-License-Identifier: MPL-2.0

#include <netinet/in.h>
#include <netinet/tcp.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <unistd.h>
#include "../common/test.h"

int sk_tcp;

FN_SETUP(general)
{
	sk_tcp = CHECK(socket(AF_INET, SOCK_STREAM, 0));
}
END_SETUP()

FN_TEST(short_optlen)
{
	int expected = 1;
	int actual = 0;
	unsigned char value[sizeof(expected)];
	socklen_t len;

	memcpy(value, &expected, sizeof(value));
	TEST_SUCC(setsockopt(sk_tcp, SOL_SOCKET, SO_KEEPALIVE, value, 2));

	len = sizeof(actual);
	TEST_RES(getsockopt(sk_tcp, SOL_SOCKET, SO_KEEPALIVE, &actual, &len),
		 actual == expected && len == sizeof(actual));

	memset(value, 0xa5, sizeof(value));
	len = 2;
	TEST_RES(getsockopt(sk_tcp, SOL_SOCKET, SO_KEEPALIVE, value, &len),
		 len == 2 && memcmp(value, &expected, len) == 0 &&
			 value[2] == 0xa5 && value[3] == 0xa5);

	memset(value, 0xa5, sizeof(value));
	len = 0;
	TEST_RES(getsockopt(sk_tcp, SOL_SOCKET, SO_KEEPALIVE, value, &len),
		 len == 0 && value[0] == 0xa5);

	unsigned char keepcnt = 5;
	int actual_keepcnt = 0;
	unsigned char keepcnt_value[sizeof(actual_keepcnt)];

	TEST_SUCC(setsockopt(sk_tcp, IPPROTO_TCP, TCP_KEEPCNT, &keepcnt,
			     sizeof(keepcnt)));

	len = sizeof(actual_keepcnt);
	TEST_RES(getsockopt(sk_tcp, IPPROTO_TCP, TCP_KEEPCNT, &actual_keepcnt,
			    &len),
		 actual_keepcnt == keepcnt && len == sizeof(actual_keepcnt));

	memset(keepcnt_value, 0xa5, sizeof(keepcnt_value));
	len = sizeof(keepcnt);
	TEST_RES(getsockopt(sk_tcp, IPPROTO_TCP, TCP_KEEPCNT, keepcnt_value,
			    &len),
		 len == sizeof(keepcnt) && keepcnt_value[0] == keepcnt &&
			 keepcnt_value[1] == 0xa5 && keepcnt_value[2] == 0xa5 &&
			 keepcnt_value[3] == 0xa5);
}
END_TEST()

FN_TEST(short_struct_optlen)
{
	struct linger linger = { .l_onoff = 1, .l_linger = 0x12345678 };
	struct linger expected_linger = { .l_onoff = linger.l_onoff };
	struct linger actual_linger = {};
	unsigned char value[sizeof(linger)];
	socklen_t len;

	TEST_SUCC(setsockopt(sk_tcp, SOL_SOCKET, SO_LINGER, &linger,
			     sizeof(linger.l_onoff)));

	len = sizeof(actual_linger);
	TEST_RES(getsockopt(sk_tcp, SOL_SOCKET, SO_LINGER, &actual_linger,
			    &len),
		 actual_linger.l_onoff == expected_linger.l_onoff &&
			 actual_linger.l_linger == expected_linger.l_linger &&
			 len == sizeof(actual_linger));

	memset(value, 0xa5, sizeof(value));
	len = sizeof(linger.l_onoff);
	TEST_RES(getsockopt(sk_tcp, SOL_SOCKET, SO_LINGER, value, &len),
		 len == sizeof(linger.l_onoff) &&
			 memcmp(value, &expected_linger, len) == 0 &&
			 value[sizeof(linger.l_onoff)] == 0xa5);

	memset(value, 0xa5, sizeof(value));
	len = 1;
	TEST_RES(getsockopt(sk_tcp, SOL_SOCKET, SO_LINGER, value, &len),
		 len == 1 &&
			 value[0] == ((unsigned char *)&expected_linger)[0] &&
			 value[1] == 0xa5);

	len = 0;
	TEST_SUCC(getsockopt(sk_tcp, SOL_SOCKET, SO_LINGER, NULL, &len));

	TEST_SUCC(setsockopt(sk_tcp, SOL_SOCKET, SO_LINGER, NULL, 0));
	memset(&actual_linger, 0xa5, sizeof(actual_linger));
	len = sizeof(actual_linger);
	TEST_RES(getsockopt(sk_tcp, SOL_SOCKET, SO_LINGER, &actual_linger,
			    &len),
		 actual_linger.l_onoff == 0 && actual_linger.l_linger == 0);

	unsigned char linger_one = 1;
	TEST_SUCC(setsockopt(sk_tcp, SOL_SOCKET, SO_LINGER, &linger_one, 1));
	memset(&actual_linger, 0, sizeof(actual_linger));
	len = sizeof(actual_linger);
	TEST_RES(getsockopt(sk_tcp, SOL_SOCKET, SO_LINGER, &actual_linger,
			    &len),
		 actual_linger.l_onoff == 1 && actual_linger.l_linger == 0);

	struct timeval timeout = { .tv_sec = 3, .tv_usec = 0x010203 };
	struct timeval expected_timeout = { .tv_sec = timeout.tv_sec };
	struct timeval actual_timeout = {};
	unsigned char timeout_value[sizeof(timeout)];

	TEST_SUCC(setsockopt(sk_tcp, SOL_SOCKET, SO_RCVTIMEO, &timeout,
			     sizeof(timeout.tv_sec)));

	len = sizeof(actual_timeout);
	TEST_RES(getsockopt(sk_tcp, SOL_SOCKET, SO_RCVTIMEO, &actual_timeout,
			    &len),
		 actual_timeout.tv_sec == expected_timeout.tv_sec &&
			 actual_timeout.tv_usec == expected_timeout.tv_usec &&
			 len == sizeof(actual_timeout));

	memset(timeout_value, 0xa5, sizeof(timeout_value));
	len = sizeof(timeout.tv_sec);
	TEST_RES(getsockopt(sk_tcp, SOL_SOCKET, SO_RCVTIMEO, timeout_value,
			    &len),
		 len == sizeof(timeout.tv_sec) &&
			 memcmp(timeout_value, &expected_timeout, len) == 0 &&
			 timeout_value[sizeof(timeout.tv_sec)] == 0xa5);

	memset(timeout_value, 0xa5, sizeof(timeout_value));
	len = 1;
	TEST_RES(getsockopt(sk_tcp, SOL_SOCKET, SO_RCVTIMEO, timeout_value,
			    &len),
		 len == 1 &&
			 timeout_value[0] ==
				 ((unsigned char *)&expected_timeout)[0] &&
			 timeout_value[1] == 0xa5);

	len = 0;
	TEST_SUCC(getsockopt(sk_tcp, SOL_SOCKET, SO_RCVTIMEO, NULL, &len));

	TEST_SUCC(setsockopt(sk_tcp, SOL_SOCKET, SO_RCVTIMEO, NULL, 0));
	memset(&actual_timeout, 0xa5, sizeof(actual_timeout));
	len = sizeof(actual_timeout);
	TEST_RES(getsockopt(sk_tcp, SOL_SOCKET, SO_RCVTIMEO, &actual_timeout,
			    &len),
		 actual_timeout.tv_sec == 0 && actual_timeout.tv_usec == 0);

	unsigned char timeout_one = 3;
	TEST_SUCC(setsockopt(sk_tcp, SOL_SOCKET, SO_RCVTIMEO, &timeout_one, 1));
	memset(&actual_timeout, 0, sizeof(actual_timeout));
	len = sizeof(actual_timeout);
	TEST_RES(getsockopt(sk_tcp, SOL_SOCKET, SO_RCVTIMEO, &actual_timeout,
			    &len),
		 actual_timeout.tv_sec == 3 && actual_timeout.tv_usec == 0);

	struct linger zero_linger = {};
	TEST_SUCC(setsockopt(sk_tcp, SOL_SOCKET, SO_LINGER, &zero_linger,
			     sizeof(zero_linger)));
	struct linger zero_actual_linger = {};
	len = sizeof(zero_actual_linger);
	TEST_RES(getsockopt(sk_tcp, SOL_SOCKET, SO_LINGER, &zero_actual_linger,
			    &len),
		 zero_actual_linger.l_onoff == 0 &&
			 zero_actual_linger.l_linger == 0);

	struct timeval zero_timeout = {};
	TEST_SUCC(setsockopt(sk_tcp, SOL_SOCKET, SO_RCVTIMEO, &zero_timeout,
			     sizeof(zero_timeout)));
	struct timeval zero_actual_timeout = {};
	len = sizeof(zero_actual_timeout);
	TEST_RES(getsockopt(sk_tcp, SOL_SOCKET, SO_RCVTIMEO,
			    &zero_actual_timeout, &len),
		 zero_actual_timeout.tv_sec == 0 &&
			 zero_actual_timeout.tv_usec == 0);
}
END_TEST()
