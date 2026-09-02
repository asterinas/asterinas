// SPDX-License-Identifier: MPL-2.0

#include <linux/netlink.h>
#include <linux/rtnetlink.h>
#include <net/if.h>
#include <stdint.h>
#include <sys/socket.h>
#include <unistd.h>

#include "../common/test.h"

struct link_request {
	struct nlmsghdr hdr;
	struct ifinfomsg ifi;
	struct nlattr attr;
	char name[NLMSG_ALIGN(sizeof("lo"))];
};

static int sock_fd;
static unsigned int loopback_index;
static union {
	struct nlmsghdr hdr;
	char bytes[8192];
} response;

FN_SETUP(netlink_socket)
{
	loopback_index = CHECK_WITH(if_nametoindex("lo"), _ret != 0);
	sock_fd = CHECK(
		socket(AF_NETLINK, SOCK_RAW | SOCK_NONBLOCK, NETLINK_ROUTE));
	struct sockaddr_nl kernel_addr = { .nl_family = AF_NETLINK };
	CHECK(connect(sock_fd, (struct sockaddr *)&kernel_addr,
		      sizeof(kernel_addr)));
}
END_SETUP()

FN_TEST(truncated_interface_name)
{
	struct link_request request = {
		.hdr = { .nlmsg_type = RTM_GETLINK,
			 .nlmsg_flags = NLM_F_REQUEST,
			 .nlmsg_seq = 1 },
		.attr = { .nla_len = NLA_HDRLEN + sizeof("lo"),
			  .nla_type = IFLA_IFNAME },
		.name = "lo",
	};
	struct link_request following = {
		.hdr = { .nlmsg_len = NLMSG_LENGTH(sizeof(struct ifinfomsg)),
			 .nlmsg_type = RTM_GETLINK,
			 .nlmsg_flags = NLM_F_REQUEST,
			 .nlmsg_seq = 2 },
		.ifi = { .ifi_index = loopback_index },
	};
	struct iovec iov[] = {
		{ &request, sizeof(request) },
		{ &following, following.hdr.nlmsg_len },
	};
	struct msghdr msg = { .msg_iov = iov, .msg_iovlen = 2 };

	/*
	 * Padding must not turn a truncated name into a valid link selector,
	 * even when truncated attributes are otherwise ignored.
	 */
	for (size_t payload_len = 1; payload_len < sizeof("lo");
	     ++payload_len) {
		request.hdr.nlmsg_len = NLMSG_LENGTH(sizeof(request.ifi)) +
					NLA_HDRLEN + payload_len;
		TEST_RES(sendmsg(sock_fd, &msg, 0),
			 _ret == (ssize_t)(iov[0].iov_len + iov[1].iov_len));
		TEST_RES(recv(sock_fd, &response, sizeof(response), 0),
			 _ret >= (ssize_t)NLMSG_LENGTH(
					 sizeof(struct nlmsgerr)) &&
				 response.hdr.nlmsg_type == NLMSG_ERROR &&
				 response.hdr.nlmsg_seq ==
					 request.hdr.nlmsg_seq &&
				 ((struct nlmsgerr *)NLMSG_DATA(&response.hdr))
						 ->error == -EINVAL);
		TEST_RES(recv(sock_fd, &response, sizeof(response), 0),
			 _ret >= (ssize_t)NLMSG_LENGTH(
					 sizeof(struct ifinfomsg)) &&
				 response.hdr.nlmsg_type == RTM_NEWLINK &&
				 response.hdr.nlmsg_seq ==
					 following.hdr.nlmsg_seq);
		TEST_ERRNO(recv(sock_fd, &response, sizeof(response), 0),
			   EAGAIN);
	}
}
END_TEST()

FN_TEST(valid_unaligned_attribute)
{
	struct link_request request = {
		.hdr = { .nlmsg_len = NLMSG_LENGTH(sizeof(struct ifinfomsg)) +
				      NLA_HDRLEN + sizeof("lo"),
			 .nlmsg_type = RTM_GETLINK,
			 .nlmsg_flags = NLM_F_REQUEST,
			 .nlmsg_seq = 3 },
		.attr = { .nla_len = NLA_HDRLEN + sizeof("lo"),
			  .nla_type = IFLA_IFNAME },
		.name = "lo",
	};

	// Final message alignment padding is optional.
	for (size_t send_len = request.hdr.nlmsg_len;
	     send_len <= NLMSG_ALIGN(request.hdr.nlmsg_len); ++send_len) {
		TEST_RES(send(sock_fd, &request, send_len, 0),
			 _ret == (ssize_t)send_len);
		TEST_RES(recv(sock_fd, &response, sizeof(response), 0),
			 _ret >= (ssize_t)NLMSG_LENGTH(
					 sizeof(struct ifinfomsg)) &&
				 response.hdr.nlmsg_type == RTM_NEWLINK &&
				 response.hdr.nlmsg_seq ==
					 request.hdr.nlmsg_seq);
	}

	// Padding must not misalign the following message header.
	struct link_request following = {
		.hdr = { .nlmsg_len = NLMSG_LENGTH(sizeof(struct ifinfomsg)),
			 .nlmsg_type = RTM_GETLINK,
			 .nlmsg_flags = NLM_F_REQUEST,
			 .nlmsg_seq = 4 },
		.ifi = { .ifi_index = loopback_index },
	};
	struct iovec iov[] = {
		{ &request, NLMSG_ALIGN(request.hdr.nlmsg_len) },
		{ &following, following.hdr.nlmsg_len },
	};
	struct msghdr msg = { .msg_iov = iov, .msg_iovlen = 2 };
	TEST_RES(sendmsg(sock_fd, &msg, 0),
		 _ret == (ssize_t)(iov[0].iov_len + iov[1].iov_len));
	TEST_RES(recv(sock_fd, &response, sizeof(response), 0),
		 _ret >= (ssize_t)NLMSG_LENGTH(sizeof(struct ifinfomsg)) &&
			 response.hdr.nlmsg_type == RTM_NEWLINK &&
			 response.hdr.nlmsg_seq == request.hdr.nlmsg_seq);
	TEST_RES(recv(sock_fd, &response, sizeof(response), 0),
		 _ret >= (ssize_t)NLMSG_LENGTH(sizeof(struct ifinfomsg)) &&
			 response.hdr.nlmsg_type == RTM_NEWLINK &&
			 response.hdr.nlmsg_seq == following.hdr.nlmsg_seq);
	TEST_ERRNO(recv(sock_fd, &response, sizeof(response), 0), EAGAIN);
}
END_TEST()

FN_TEST(invalid_body_with_padding)
{
	int strict = 1;
	TEST_SUCC(setsockopt(sock_fd, SOL_NETLINK, NETLINK_GET_STRICT_CHK,
			     &strict, sizeof(strict)));

	struct link_request request = {
		.hdr = { .nlmsg_len = NLMSG_LENGTH(sizeof(struct ifinfomsg)) +
				      NLA_HDRLEN + sizeof("lo"),
			 .nlmsg_type = RTM_GETLINK,
			 .nlmsg_flags = NLM_F_REQUEST,
			 .nlmsg_seq = 5 },
		.ifi = { .ifi_type = UINT16_MAX, .ifi_index = loopback_index },
		.attr = { .nla_len = NLA_HDRLEN + sizeof("lo"),
			  .nla_type = IFLA_IFNAME },
		.name = "lo",
	};
	struct link_request following = {
		.hdr = { .nlmsg_len = NLMSG_LENGTH(sizeof(struct ifinfomsg)),
			 .nlmsg_type = RTM_GETLINK,
			 .nlmsg_flags = NLM_F_REQUEST,
			 .nlmsg_seq = 6 },
		.ifi = { .ifi_index = loopback_index },
	};
	struct iovec iov[] = {
		{ &request, NLMSG_ALIGN(request.hdr.nlmsg_len) },
		{ &following, following.hdr.nlmsg_len },
	};
	struct msghdr msg = { .msg_iov = iov, .msg_iovlen = 2 };
	// Body validation failure must preserve the next message boundary.
	TEST_RES(sendmsg(sock_fd, &msg, 0),
		 _ret == (ssize_t)(iov[0].iov_len + iov[1].iov_len));
	TEST_RES(
		recv(sock_fd, &response, sizeof(response), 0),
		_ret >= (ssize_t)NLMSG_LENGTH(sizeof(struct nlmsgerr)) &&
			response.hdr.nlmsg_type == NLMSG_ERROR &&
			response.hdr.nlmsg_seq == request.hdr.nlmsg_seq &&
			((struct nlmsgerr *)NLMSG_DATA(&response.hdr))->error ==
				-EINVAL);
	TEST_RES(recv(sock_fd, &response, sizeof(response), 0),
		 _ret >= (ssize_t)NLMSG_LENGTH(sizeof(struct ifinfomsg)) &&
			 response.hdr.nlmsg_type == RTM_NEWLINK &&
			 response.hdr.nlmsg_seq == following.hdr.nlmsg_seq);
	TEST_ERRNO(recv(sock_fd, &response, sizeof(response), 0), EAGAIN);

	strict = 0;
	TEST_SUCC(setsockopt(sock_fd, SOL_NETLINK, NETLINK_GET_STRICT_CHK,
			     &strict, sizeof(strict)));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(sock_fd));
}
END_SETUP()

FN_TEST(strict_truncated_attribute_payload)
{
	int strict_fd = TEST_SUCC(
		socket(AF_NETLINK, SOCK_RAW | SOCK_NONBLOCK, NETLINK_ROUTE));
	struct sockaddr_nl kernel_addr = { .nl_family = AF_NETLINK };
	TEST_SUCC(connect(strict_fd, (struct sockaddr *)&kernel_addr,
			  sizeof(kernel_addr)));
	int strict = 1;
	TEST_SUCC(setsockopt(strict_fd, SOL_NETLINK, NETLINK_GET_STRICT_CHK,
			     &strict, sizeof(strict)));

	struct {
		struct nlmsghdr hdr;
		struct ifinfomsg ifi;
		struct nlattr attr;
		uint32_t mask;
	} request = {
		.hdr = { .nlmsg_type = RTM_GETLINK,
			 .nlmsg_flags = NLM_F_REQUEST,
			 .nlmsg_seq = 7 },
		.ifi = { .ifi_index = loopback_index },
		.attr = { .nla_len = NLA_HDRLEN + sizeof(uint32_t),
			  .nla_type = IFLA_EXT_MASK },
	};
	struct link_request following = {
		.hdr = { .nlmsg_len = NLMSG_LENGTH(sizeof(struct ifinfomsg)),
			 .nlmsg_type = RTM_GETLINK,
			 .nlmsg_flags = NLM_F_REQUEST,
			 .nlmsg_seq = 8 },
		.ifi = { .ifi_index = loopback_index },
	};
	struct iovec iov[] = {
		{ &request, sizeof(request) },
		{ &following, following.hdr.nlmsg_len },
	};
	struct msghdr msg = { .msg_iov = iov, .msg_iovlen = 2 };

	// Padding must not supply the missing bytes of the four-byte payload.
	for (size_t payload_len = 1; payload_len < sizeof(uint32_t);
	     ++payload_len) {
		request.hdr.nlmsg_len = NLMSG_LENGTH(sizeof(request.ifi)) +
					NLA_HDRLEN + payload_len;
		TEST_RES(sendmsg(strict_fd, &msg, 0),
			 _ret == (ssize_t)(iov[0].iov_len + iov[1].iov_len));
		TEST_RES(recv(strict_fd, &response, sizeof(response), 0),
			 _ret >= (ssize_t)NLMSG_LENGTH(
					 sizeof(struct nlmsgerr)) &&
				 response.hdr.nlmsg_type == NLMSG_ERROR &&
				 response.hdr.nlmsg_seq ==
					 request.hdr.nlmsg_seq &&
				 ((struct nlmsgerr *)NLMSG_DATA(&response.hdr))
						 ->error == -EINVAL);
		TEST_RES(recv(strict_fd, &response, sizeof(response), 0),
			 _ret >= (ssize_t)NLMSG_LENGTH(
					 sizeof(struct ifinfomsg)) &&
				 response.hdr.nlmsg_type == RTM_NEWLINK &&
				 response.hdr.nlmsg_seq ==
					 following.hdr.nlmsg_seq);
		TEST_ERRNO(recv(strict_fd, &response, sizeof(response), 0),
			   EAGAIN);
	}
	TEST_SUCC(close(strict_fd));
}
END_TEST()
