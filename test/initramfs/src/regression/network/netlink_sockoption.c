// SPDX-License-Identifier: MPL-2.0

#include <linux/netlink.h>
#include <stdint.h>
#include <sys/socket.h>
#include <unistd.h>

#include "../common/test.h"

static const int protocols[] = { NETLINK_ROUTE, NETLINK_KOBJECT_UEVENT };

FN_TEST(membership_group_ids)
{
	const uint32_t group_ids[] = { 5, 32 };
	for (size_t p = 0; p < sizeof(protocols) / sizeof(protocols[0]); ++p) {
		int sock_fd =
			TEST_SUCC(socket(AF_NETLINK, SOCK_RAW, protocols[p]));
		uint32_t first_group = 1;
		TEST_SUCC(setsockopt(sock_fd, SOL_NETLINK,
				     NETLINK_ADD_MEMBERSHIP, &first_group,
				     sizeof(first_group)));
		struct sockaddr_nl addr;
		socklen_t addr_len = sizeof(addr);
		TEST_RES(getsockname(sock_fd, (struct sockaddr *)&addr,
				     &addr_len),
			 addr.nl_groups == 1 && addr.nl_pid == 0);

		for (int bound = 0; bound < 2; ++bound) {
			if (bound) {
				struct sockaddr_nl bind_addr = {
					.nl_family = AF_NETLINK,
					.nl_groups = 1,
				};
				TEST_SUCC(bind(sock_fd,
					       (struct sockaddr *)&bind_addr,
					       sizeof(bind_addr)));
			}

			for (size_t i = 0;
			     i < sizeof(group_ids) / sizeof(group_ids[0]);
			     ++i) {
				uint32_t group = group_ids[i];
				uint32_t mask = UINT32_C(1) << (group - 1);
				TEST_SUCC(setsockopt(sock_fd, SOL_NETLINK,
						     NETLINK_ADD_MEMBERSHIP,
						     &group, sizeof(group)));
				TEST_RES(getsockname(sock_fd,
						     (struct sockaddr *)&addr,
						     &addr_len),
					 addr.nl_groups == (1 | mask));

				TEST_SUCC(setsockopt(sock_fd, SOL_NETLINK,
						     NETLINK_DROP_MEMBERSHIP,
						     &first_group,
						     sizeof(first_group)));
				TEST_RES(getsockname(sock_fd,
						     (struct sockaddr *)&addr,
						     &addr_len),
					 addr.nl_groups == mask);
				TEST_SUCC(setsockopt(sock_fd, SOL_NETLINK,
						     NETLINK_DROP_MEMBERSHIP,
						     &group, sizeof(group)));
				TEST_RES(getsockname(sock_fd,
						     (struct sockaddr *)&addr,
						     &addr_len),
					 addr.nl_groups == 0);
				TEST_SUCC(setsockopt(sock_fd, SOL_NETLINK,
						     NETLINK_ADD_MEMBERSHIP,
						     &first_group,
						     sizeof(first_group)));
			}
		}
		TEST_SUCC(close(sock_fd));
	}
}
END_TEST()

FN_TEST(invalid_membership)
{
	const int options[] = { NETLINK_ADD_MEMBERSHIP,
				NETLINK_DROP_MEMBERSHIP };
	const uint32_t invalid_groups[] = { 0, UINT32_MAX, UINT32_C(1) << 31 };
	const socklen_t output_lengths[] = { 0, sizeof(uint32_t) };
	for (size_t p = 0; p < sizeof(protocols) / sizeof(protocols[0]); ++p) {
		int sock_fd =
			TEST_SUCC(socket(AF_NETLINK, SOCK_RAW, protocols[p]));
		struct sockaddr_nl addr = { .nl_family = AF_NETLINK,
					    .nl_groups = 1 };
		TEST_SUCC(
			bind(sock_fd, (struct sockaddr *)&addr, sizeof(addr)));
		socklen_t addr_len = sizeof(addr);
		for (size_t i = 0; i < sizeof(options) / sizeof(options[0]);
		     ++i) {
			uint32_t group = UINT32_MAX;
			for (socklen_t len = 0; len < sizeof(group); ++len) {
				TEST_ERRNO(setsockopt(sock_fd, SOL_NETLINK,
						      options[i], &group, len),
					   EINVAL);
				TEST_ERRNO(setsockopt(sock_fd, SOL_NETLINK,
						      options[i], NULL, len),
					   EINVAL);
				TEST_RES(getsockname(sock_fd,
						     (struct sockaddr *)&addr,
						     &addr_len),
					 addr.nl_groups == 1);
			}
			for (size_t j = 0;
			     j <
			     sizeof(invalid_groups) / sizeof(invalid_groups[0]);
			     ++j) {
				TEST_ERRNO(setsockopt(sock_fd, SOL_NETLINK,
						      options[i],
						      &invalid_groups[j],
						      sizeof(group)),
					   EINVAL);
				TEST_RES(getsockname(sock_fd,
						     (struct sockaddr *)&addr,
						     &addr_len),
					 addr.nl_groups == 1);
			}
			TEST_ERRNO(setsockopt(sock_fd, SOL_NETLINK, options[i],
					      NULL, sizeof(group)),
				   EFAULT);

			// Membership options remain setter-only regardless of output length.
			for (size_t j = 0;
			     j <
			     sizeof(output_lengths) / sizeof(output_lengths[0]);
			     ++j) {
				socklen_t output_len = output_lengths[j];
				TEST_ERRNO(getsockopt(sock_fd, SOL_NETLINK,
						      options[i], &group,
						      &output_len),
					   ENOPROTOOPT);
			}
		}
		TEST_SUCC(close(sock_fd));
	}
}
END_TEST()

FN_TEST(strict_check_socket_option)
{
	for (size_t p = 0; p < sizeof(protocols) / sizeof(protocols[0]); ++p) {
		int sock_fd =
			TEST_SUCC(socket(AF_NETLINK, SOCK_RAW, protocols[p]));

		int strict = -1;
		socklen_t option_len = sizeof(strict);

		TEST_RES(getsockopt(sock_fd, SOL_NETLINK,
				    NETLINK_GET_STRICT_CHK, &strict,
				    &option_len),
			 strict == 0 && option_len == sizeof(strict));

		strict = 1;
		TEST_SUCC(setsockopt(sock_fd, SOL_NETLINK,
				     NETLINK_GET_STRICT_CHK, &strict,
				     sizeof(strict)));
		strict = 0;
		option_len = sizeof(strict);
		TEST_RES(getsockopt(sock_fd, SOL_NETLINK,
				    NETLINK_GET_STRICT_CHK, &strict,
				    &option_len),
			 strict == 1 && option_len == sizeof(strict));

		strict = 0;
		TEST_SUCC(setsockopt(sock_fd, SOL_NETLINK,
				     NETLINK_GET_STRICT_CHK, &strict,
				     sizeof(strict)));
		option_len = sizeof(strict);
		TEST_RES(getsockopt(sock_fd, SOL_NETLINK,
				    NETLINK_GET_STRICT_CHK, &strict,
				    &option_len),
			 strict == 0 && option_len == sizeof(strict));

		TEST_ERRNO(getsockopt(sock_fd, SOL_NETLINK, 9999, &strict,
				      &option_len),
			   ENOPROTOOPT);

		for (socklen_t len = 0; len < sizeof(int); ++len) {
			int enabled = 1;
			TEST_SUCC(setsockopt(sock_fd, SOL_NETLINK,
					     NETLINK_GET_STRICT_CHK, &enabled,
					     sizeof(enabled)));
			uint32_t input = UINT32_MAX;
			TEST_SUCC(setsockopt(sock_fd, SOL_NETLINK,
					     NETLINK_GET_STRICT_CHK, &input,
					     len));
			int value = -1;
			socklen_t value_len = sizeof(value);
			TEST_RES(getsockopt(sock_fd, SOL_NETLINK,
					    NETLINK_GET_STRICT_CHK, &value,
					    &value_len),
				 value == 0 && value_len == sizeof(value));

			// Short inputs must not access even a NULL buffer.
			TEST_SUCC(setsockopt(sock_fd, SOL_NETLINK,
					     NETLINK_GET_STRICT_CHK, &enabled,
					     sizeof(enabled)));
			TEST_SUCC(setsockopt(sock_fd, SOL_NETLINK,
					     NETLINK_GET_STRICT_CHK, NULL,
					     len));
			value = -1;
			value_len = sizeof(value);
			TEST_RES(getsockopt(sock_fd, SOL_NETLINK,
					    NETLINK_GET_STRICT_CHK, &value,
					    &value_len),
				 value == 0 && value_len == sizeof(value));

			uint32_t sentinel = 0xa5a5a5a5;
			uint32_t output = sentinel;
			value_len = len;
			TEST_ERRNO(getsockopt(sock_fd, SOL_NETLINK,
					      NETLINK_GET_STRICT_CHK, &output,
					      &value_len),
				   EINVAL);
			TEST_RES(memcmp(&output, &sentinel, sizeof(output)),
				 _ret == 0 && value_len == len);
			TEST_ERRNO(getsockopt(sock_fd, SOL_NETLINK,
					      NETLINK_GET_STRICT_CHK, NULL,
					      &value_len),
				   EINVAL);
		}

		// Ignore extra input bytes and only overwrite one integer on output.
		int input[2] = { -1, 0 };
		TEST_SUCC(setsockopt(sock_fd, SOL_NETLINK,
				     NETLINK_GET_STRICT_CHK, input,
				     sizeof(input)));
		TEST_ERRNO(setsockopt(sock_fd, SOL_NETLINK,
				      NETLINK_GET_STRICT_CHK, NULL,
				      sizeof(int)),
			   EFAULT);
		int output[2] = { -1, -1 };
		socklen_t output_len = sizeof(output);
		TEST_RES(getsockopt(sock_fd, SOL_NETLINK,
				    NETLINK_GET_STRICT_CHK, output,
				    &output_len),
			 output[0] == 1 && output[1] == -1 &&
				 output_len == sizeof(int));
		TEST_SUCC(close(sock_fd));
	}
}
END_TEST()
