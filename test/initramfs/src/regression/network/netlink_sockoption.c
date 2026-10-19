// SPDX-License-Identifier: MPL-2.0

#include <linux/netlink.h>
#include <stdint.h>
#include <sys/socket.h>
#include <unistd.h>

#include "netlink_sockoption.h"

static const int protocols[] = { NETLINK_ROUTE, NETLINK_KOBJECT_UEVENT };

FN_TEST(membership_group_ids)
{
	const uint32_t group_ids[] = { 5, 32 };
	for (size_t p = 0; p < sizeof(protocols) / sizeof(protocols[0]); ++p) {
		int sock_fd =
			TEST_SUCC(socket(AF_NETLINK, SOCK_RAW, protocols[p]));
		TEST_SETSOCKOPT_INT(sock_fd, NETLINK_ADD_MEMBERSHIP, 1);
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
				TEST_SETSOCKOPT_INT(
					sock_fd, NETLINK_ADD_MEMBERSHIP, group);
				TEST_RES(getsockname(sock_fd,
						     (struct sockaddr *)&addr,
						     &addr_len),
					 addr.nl_groups == (1 | mask));

				TEST_SETSOCKOPT_INT(sock_fd,
						    NETLINK_DROP_MEMBERSHIP, 1);
				TEST_RES(getsockname(sock_fd,
						     (struct sockaddr *)&addr,
						     &addr_len),
					 addr.nl_groups == mask);
				TEST_SETSOCKOPT_INT(sock_fd,
						    NETLINK_DROP_MEMBERSHIP,
						    group);
				TEST_RES(getsockname(sock_fd,
						     (struct sockaddr *)&addr,
						     &addr_len),
					 addr.nl_groups == 0);
				TEST_SETSOCKOPT_INT(sock_fd,
						    NETLINK_ADD_MEMBERSHIP, 1);
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
