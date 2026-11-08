// SPDX-License-Identifier: MPL-2.0

#include <linux/netlink.h>
#include <linux/rtnetlink.h>
#include <net/if.h>
#include <stdint.h>
#include <sys/socket.h>
#include <unistd.h>

#include "netlink_sockoption.h"

struct link_request {
	struct nlmsghdr hdr;
	struct ifinfomsg ifi;
	struct nlattr attr;
	union {
		char name[NLMSG_ALIGN(sizeof("lo"))];
		uint32_t mask;
	};
};

static int sock_fd;
static unsigned int loopback_index;
static union {
	struct nlmsghdr hdr;
	char bytes[8192];
} response;

static const struct link_request name_request = {
	.hdr = { .nlmsg_len = NLMSG_LENGTH(sizeof(struct ifinfomsg)) +
			      NLA_HDRLEN + sizeof("lo"),
		 .nlmsg_type = RTM_GETLINK,
		 .nlmsg_flags = NLM_F_REQUEST,
		 .nlmsg_seq = 1 },
	.attr = { .nla_len = NLA_HDRLEN + sizeof("lo"),
		  .nla_type = IFLA_IFNAME },
	.name = "lo",
};

static struct link_request following = {
	.hdr = { .nlmsg_len = NLMSG_LENGTH(sizeof(struct ifinfomsg)),
		 .nlmsg_type = RTM_GETLINK,
		 .nlmsg_flags = NLM_F_REQUEST,
		 .nlmsg_seq = 2 },
};

FN_SETUP(netlink_socket)
{
	loopback_index = CHECK_WITH(if_nametoindex("lo"), _ret != 0);
	following.ifi.ifi_index = loopback_index;

	sock_fd = CHECK(
		socket(AF_NETLINK, SOCK_RAW | SOCK_NONBLOCK, NETLINK_ROUTE));

	struct sockaddr_nl kernel_addr = { .nl_family = AF_NETLINK };
	CHECK(connect(sock_fd, (struct sockaddr *)&kernel_addr,
		      sizeof(kernel_addr)));
}
END_SETUP()

// request_len includes exactly the padding bytes exercised by the test.
static int send_request(int fd, struct nlmsghdr *request, size_t request_len,
			struct nlmsghdr *following)
{
	struct iovec iov[] = {
		{ request, request_len },
		{ following, following ? following->nlmsg_len : 0 },
	};
	struct msghdr msg = {
		.msg_iov = iov,
		.msg_iovlen = following ? 2 : 1,
	};
	ssize_t sent = sendmsg(fd, &msg, 0);
	if (sent < 0)
		return -1;
	if (sent != (ssize_t)(iov[0].iov_len + iov[1].iov_len)) {
		errno = EIO;
		return -1;
	}
	return 0;
}

#define __TEST_RECV(fd, request, msg_type, payload_type, cond)                \
	TEST_RES(recv(fd, &response, sizeof(response), 0),                    \
		 _ret >= (ssize_t)NLMSG_LENGTH(sizeof(payload_type)) &&       \
			 NLMSG_OK(&response.hdr, _ret) &&                     \
			 response.hdr.nlmsg_len >=                            \
				 NLMSG_LENGTH(sizeof(payload_type)) &&        \
			 response.hdr.nlmsg_type == (msg_type) &&             \
			 response.hdr.nlmsg_seq == (request).hdr.nlmsg_seq && \
			 (cond))

#define TEST_RECV_ERRNO(fd, request, expected_errno)                         \
	__TEST_RECV(fd, request, NLMSG_ERROR, struct nlmsgerr,               \
		    ((struct nlmsgerr *)NLMSG_DATA(&response.hdr))->error == \
			    -(expected_errno))

#define TEST_RECV_LINK(fd, request) \
	__TEST_RECV(fd, request, RTM_NEWLINK, struct ifinfomsg, 1)

#define TEST_RECV_EMPTY(fd) \
	TEST_ERRNO(recv(fd, &response, sizeof(response), 0), EAGAIN)

FN_TEST(truncated_attribute_payload)
{
	struct link_request request;

	for (int strict = 0; strict < 2; ++strict) {
		TEST_SETSOCKOPT_INT(sock_fd, NETLINK_GET_STRICT_CHK, strict);

		size_t attr_payload_len;
		if (!strict) {
			/*
			 * Without strict checking, the truncated name is ignored
			 * and the request fails because it has neither a valid
			 * name nor an interface index.
			 */
			request = name_request;
			attr_payload_len = sizeof("lo");
		} else {
			/*
			 * Strict checking must reject the truncated attribute even
			 * with a valid interface index. Ignoring it would succeed.
			 */
			request.ifi.ifi_index = loopback_index;
			request.attr.nla_type = IFLA_EXT_MASK;
			request.mask = 0;
			attr_payload_len = sizeof(request.mask);
		}
		request.attr.nla_len = NLA_HDRLEN + attr_payload_len;

		// Padding must not supply the missing attribute payload bytes.
		for (size_t payload_len = 1; payload_len < attr_payload_len;
		     ++payload_len) {
			request.hdr.nlmsg_len =
				NLMSG_LENGTH(sizeof(request.ifi)) + NLA_HDRLEN +
				payload_len;
			TEST_SUCC(send_request(sock_fd, &request.hdr,
					       sizeof(request),
					       &following.hdr));
			TEST_RECV_ERRNO(sock_fd, request, EINVAL);
			TEST_RECV_LINK(sock_fd, following);
			TEST_RECV_EMPTY(sock_fd);
		}
	}

	TEST_SETSOCKOPT_INT(sock_fd, NETLINK_GET_STRICT_CHK, 0);
}
END_TEST()

FN_TEST(valid_unaligned_attribute)
{
	struct link_request request = name_request;

	// Final message alignment padding is optional.
	for (size_t send_len = request.hdr.nlmsg_len;
	     send_len <= NLMSG_ALIGN(request.hdr.nlmsg_len); ++send_len) {
		TEST_SUCC(send_request(sock_fd, &request.hdr, send_len, NULL));
		TEST_RECV_LINK(sock_fd, request);
	}

	// Padding must not misalign the following message header.
	TEST_SUCC(send_request(sock_fd, &request.hdr,
			       NLMSG_ALIGN(request.hdr.nlmsg_len),
			       &following.hdr));
	TEST_RECV_LINK(sock_fd, request);
	TEST_RECV_LINK(sock_fd, following);
	TEST_RECV_EMPTY(sock_fd);

	TEST_SETSOCKOPT_INT(sock_fd, NETLINK_GET_STRICT_CHK, 1);

	// Invalid ifi_type.
	request.ifi.ifi_type = UINT16_MAX;

	// Padding must not misalign the following message header,
	// even if body validation fails.
	TEST_SUCC(send_request(sock_fd, &request.hdr,
			       NLMSG_ALIGN(request.hdr.nlmsg_len),
			       &following.hdr));
	TEST_RECV_ERRNO(sock_fd, request, EINVAL);
	TEST_RECV_LINK(sock_fd, following);
	TEST_RECV_EMPTY(sock_fd);

	TEST_SETSOCKOPT_INT(sock_fd, NETLINK_GET_STRICT_CHK, 0);
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(sock_fd));
}
END_SETUP()
