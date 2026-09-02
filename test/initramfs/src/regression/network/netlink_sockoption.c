// SPDX-License-Identifier: MPL-2.0

#include <linux/netlink.h>
#include <stdint.h>
#include <sys/socket.h>
#include <unistd.h>

#include "netlink_sockoption.h"

static const int protocols[] = { NETLINK_ROUTE, NETLINK_KOBJECT_UEVENT };

#define FN_TEST_FOREACH_PROTO(name, sock_fd)                              \
	FN_TEST(name)                                                     \
	{                                                                 \
		for (size_t p = 0;                                        \
		     p < sizeof(protocols) / sizeof(protocols[0]); ++p) { \
			int sock_fd = TEST_SUCC(                          \
				socket(AF_NETLINK, SOCK_RAW, protocols[p]));

#define END_TEST_FOREACH_PROTO(sock_fd) \
	TEST_SUCC(close(sock_fd));      \
	}                               \
	}                               \
	END_TEST()

#define TEST_NL_GROUPS(fd, expected_nl_groups)                             \
	do {                                                               \
		struct sockaddr_nl _addr;                                  \
		socklen_t _addr_len = sizeof(_addr);                       \
		TEST_RES(getsockname(fd, (struct sockaddr *)&_addr,        \
				     &_addr_len),                          \
			 _addr_len == sizeof(_addr) &&                     \
				 _addr.nl_groups == (expected_nl_groups)); \
	} while (0)

FN_TEST_FOREACH_PROTO(membership_group_ids, sock_fd)
{
	TEST_SETSOCKOPT_INT(sock_fd, NETLINK_ADD_MEMBERSHIP, 1);
	TEST_NL_GROUPS(sock_fd, 1);

	for (int bound = 0; bound < 2; ++bound) {
		if (bound) {
			struct sockaddr_nl bind_addr = {
				.nl_family = AF_NETLINK,
				.nl_groups = 1,
			};
			TEST_SUCC(bind(sock_fd, (struct sockaddr *)&bind_addr,
				       sizeof(bind_addr)));
		}

		const uint32_t group_ids[] = { 5, 32 };
		for (size_t i = 0; i < sizeof(group_ids) / sizeof(group_ids[0]);
		     ++i) {
			uint32_t group = group_ids[i];
			uint32_t mask = UINT32_C(1) << (group - 1);

			TEST_SETSOCKOPT_INT(sock_fd, NETLINK_ADD_MEMBERSHIP,
					    group);
			TEST_NL_GROUPS(sock_fd, 1 | mask);

			TEST_SETSOCKOPT_INT(sock_fd, NETLINK_DROP_MEMBERSHIP,
					    1);
			TEST_NL_GROUPS(sock_fd, mask);

			TEST_SETSOCKOPT_INT(sock_fd, NETLINK_DROP_MEMBERSHIP,
					    group);
			TEST_NL_GROUPS(sock_fd, 0);

			TEST_SETSOCKOPT_INT(sock_fd, NETLINK_ADD_MEMBERSHIP, 1);
		}
	}
}
END_TEST_FOREACH_PROTO(sock_fd)

FN_TEST_FOREACH_PROTO(invalid_membership, sock_fd)
{
	struct sockaddr_nl addr = { .nl_family = AF_NETLINK, .nl_groups = 1 };
	TEST_SUCC(bind(sock_fd, (struct sockaddr *)&addr, sizeof(addr)));

	const int options[] = { NETLINK_ADD_MEMBERSHIP,
				NETLINK_DROP_MEMBERSHIP };
	for (size_t i = 0; i < sizeof(options) / sizeof(options[0]); ++i) {
		uint32_t group = 5;
		for (socklen_t len = 0; len < sizeof(group); ++len) {
			// Setting truncated membership options fails with EINVAL.
			TEST_ERRNO(setsockopt(sock_fd, SOL_NETLINK, options[i],
					      &group, len),
				   EINVAL);
			TEST_ERRNO(setsockopt(sock_fd, SOL_NETLINK, options[i],
					      NULL, len),
				   EINVAL);

			TEST_NL_GROUPS(sock_fd, 1);
		}
		// Setting membership options with a NULL pointer fails with EFAULT.
		TEST_ERRNO(setsockopt(sock_fd, SOL_NETLINK, options[i], NULL,
				      sizeof(group)),
			   EFAULT);

		const uint32_t invalid_groups[] = { 0, UINT32_MAX,
						    UINT32_C(1) << 31 };
		for (size_t j = 0;
		     j < sizeof(invalid_groups) / sizeof(invalid_groups[0]);
		     ++j) {
			// Setting invalid membership options fails with EINVAL.
			TEST_ERRNO(setsockopt(sock_fd, SOL_NETLINK, options[i],
					      &invalid_groups[j],
					      sizeof(group)),
				   EINVAL);

			TEST_NL_GROUPS(sock_fd, 1);
		}

		for (socklen_t len = 0; len <= sizeof(group); ++len) {
			// Getting membership options fails with ENOPROTOOPT.
			TEST_ERRNO(getsockopt(sock_fd, SOL_NETLINK, options[i],
					      &group, &len),
				   ENOPROTOOPT);
		}
	}
}
END_TEST_FOREACH_PROTO(sock_fd)

FN_TEST_FOREACH_PROTO(strict_check_socket_option, sock_fd)
{
	TEST_GETSOCKOPT_INT(sock_fd, NETLINK_GET_STRICT_CHK, _opt == 0);

	TEST_SETSOCKOPT_INT(sock_fd, NETLINK_GET_STRICT_CHK, 1);
	TEST_GETSOCKOPT_INT(sock_fd, NETLINK_GET_STRICT_CHK, _opt == 1);
	TEST_SETSOCKOPT_INT(sock_fd, NETLINK_GET_STRICT_CHK, 0);
	TEST_GETSOCKOPT_INT(sock_fd, NETLINK_GET_STRICT_CHK, _opt == 0);

	int strict = -1;
	socklen_t option_len = sizeof(strict);
	TEST_ERRNO(getsockopt(sock_fd, SOL_NETLINK, 9999, &strict, &option_len),
		   ENOPROTOOPT);

	for (socklen_t len = 0; len < sizeof(int); ++len) {
		uint32_t input = UINT32_MAX;

		TEST_SETSOCKOPT_INT(sock_fd, NETLINK_GET_STRICT_CHK, 1);
		TEST_SUCC(setsockopt(sock_fd, SOL_NETLINK,
				     NETLINK_GET_STRICT_CHK, &input, len));
		TEST_GETSOCKOPT_INT(sock_fd, NETLINK_GET_STRICT_CHK, _opt == 0);

		// Short inputs must not access even a NULL buffer.
		TEST_SETSOCKOPT_INT(sock_fd, NETLINK_GET_STRICT_CHK, 1);
		TEST_SUCC(setsockopt(sock_fd, SOL_NETLINK,
				     NETLINK_GET_STRICT_CHK, NULL, len));
		TEST_GETSOCKOPT_INT(sock_fd, NETLINK_GET_STRICT_CHK, _opt == 0);

		uint32_t sentinel = 0xa5a5a5a5;
		uint32_t output = sentinel;
		socklen_t value_len = len;
		TEST_ERRNO(getsockopt(sock_fd, SOL_NETLINK,
				      NETLINK_GET_STRICT_CHK, &output,
				      &value_len),
			   EINVAL);
		TEST_RES(memcmp(&output, &sentinel, sizeof(output)),
			 _ret == 0 && value_len == len);
		TEST_ERRNO(getsockopt(sock_fd, SOL_NETLINK,
				      NETLINK_GET_STRICT_CHK, NULL, &value_len),
			   EINVAL);
	}

	// Ignore extra input bytes and only overwrite one integer on output.
	int input[2] = { -1, 0 };
	TEST_SUCC(setsockopt(sock_fd, SOL_NETLINK, NETLINK_GET_STRICT_CHK,
			     input, sizeof(input)));
	TEST_ERRNO(setsockopt(sock_fd, SOL_NETLINK, NETLINK_GET_STRICT_CHK,
			      NULL, sizeof(int)),
		   EFAULT);
	int output[2] = { -1, -1 };
	socklen_t output_len = sizeof(output);
	TEST_RES(getsockopt(sock_fd, SOL_NETLINK, NETLINK_GET_STRICT_CHK,
			    output, &output_len),
		 output[0] == 1 && output[1] == -1 &&
			 output_len == sizeof(int));
}
END_TEST_FOREACH_PROTO(sock_fd)
