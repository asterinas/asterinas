// SPDX-License-Identifier: MPL-2.0

#include <net/if.h>
#include <netlink/route/addr.h>
#include <unistd.h>

#include "../test.h"

#define ETHER_NAME "eth0"
#define LOOPBACK_NAME "lo"

#define SUCC(expr) ((expr), 0)

int find_lo_and_eth0_by_libc(struct if_nameindex *if_ni)
{
	int found_links = 0;

	for (struct if_nameindex *i = if_ni;
	     !(i->if_index == 0 && i->if_name == NULL); i++) {
		if (strcmp(i->if_name, LOOPBACK_NAME) == 0) {
			found_links++;
		}

		if (strcmp(i->if_name, ETHER_NAME) == 0) {
			found_links++;
		}
	}

	return found_links;
}

FN_TEST(if_nameindex)
{
	struct if_nameindex *if_ni;

	CHECK_WITH(SUCC(if_ni = if_nameindex()), if_ni != NULL);

	TEST_RES(find_lo_and_eth0_by_libc(if_ni), _ret == 2);

	if_freenameindex(if_ni);
}
END_TEST()

void find_lo_and_eth0_by_libnl(struct nl_object *obj, void *arg)
{
	int *found_links = (int *)arg;
	struct rtnl_link *link = (struct rtnl_link *)obj;

	if (strcmp(rtnl_link_get_name(link), LOOPBACK_NAME) == 0) {
		*found_links += 1;
	}

	if (strcmp(rtnl_link_get_name(link), ETHER_NAME) == 0) {
		*found_links += 1;
	}
}

FN_TEST(get_link_by_libnl)
{
	struct nl_sock *sock;
	struct nl_cache *link_cache;

	// 1. Create netlink socket and connect
	sock = nl_socket_alloc();
	TEST_RES(nl_connect(sock, NETLINK_ROUTE), _ret >= 0);

	// 2. Allocate and retrieve link cache
	TEST_RES(rtnl_link_alloc_cache(sock, AF_UNSPEC, &link_cache),
		 _ret >= 0);

	// 3. Iterate over all links to find lo and eth0
	int found_links = 0;
	TEST_RES(SUCC(nl_cache_foreach(link_cache, find_lo_and_eth0_by_libnl,
				       &found_links)),
		 found_links == 2);

	// 4. Cleanup
	nl_cache_free(link_cache);
	nl_close(sock);
	nl_socket_free(sock);
}
END_TEST()

void find_loopback_address(struct nl_object *obj, void *arg)
{
	int *found_loopback = (int *)arg;
	struct rtnl_addr *addr = (struct rtnl_addr *)obj;
	struct nl_addr *local;
	char buf[INET_ADDRSTRLEN];

	int family = rtnl_addr_get_family(addr);
	if (family != AF_INET) {
		return;
	}

	local = rtnl_addr_get_local(addr);
	if (local) {
		nl_addr2str(local, buf, sizeof(buf));
		if (strcmp(buf, "127.0.0.1/8") == 0) {
			*found_loopback = 1;
		}
	}
}

FN_TEST(get_loopback_address)
{
	struct nl_sock *sock;
	struct nl_cache *addr_cache;

	// 1. Create netlink socket and connect
	sock = nl_socket_alloc();
	TEST_RES(nl_connect(sock, NETLINK_ROUTE), _ret >= 0);

	// 2. Allocate and retrieve address cache
	TEST_RES(rtnl_addr_alloc_cache(sock, &addr_cache), _ret >= 0);

	// 3. Iterate over all addresses to find loopback address
	int found_loopback = 0;
	TEST_RES(SUCC(nl_cache_foreach(addr_cache, find_loopback_address,
				       &found_loopback)),
		 found_loopback == 1);

	// 4. Cleanup
	nl_cache_free(addr_cache);
	nl_close(sock);
	nl_socket_free(sock);
}
END_TEST()

int find_new_addr_until_done(char *buffer, size_t len, int *found_new_addr)
{
	struct nlmsghdr *nlh = (struct nlmsghdr *)buffer;

	for (; NLMSG_OK(nlh, len); nlh = NLMSG_NEXT(nlh, len)) {
		if (nlh->nlmsg_type == NLMSG_DONE) {
			return *found_new_addr ? 1 : -1;
		}

		if (nlh->nlmsg_type == RTM_NEWADDR) {
			*found_new_addr += 1;
		} else {
			return -1;
		}
	}

	return 0;
}

#define BUFFER_SIZE 8192
char buffer[BUFFER_SIZE];

FN_TEST(get_link_error)
{
	int sock_fd;
	struct sockaddr_nl sa;

	sock_fd = TEST_SUCC(socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE));

	memset(&sa, 0, sizeof(sa));
	sa.nl_family = AF_NETLINK;

	TEST_SUCC(bind(sock_fd, (struct sockaddr *)&sa, sizeof(sa)));

	struct nl_req {
		struct nlmsghdr hdr;
		struct ifinfomsg ifi;
		struct nlattr ahdr;
		char abuf[IFNAMSIZ];
	};

	struct nl_req req;
	memset(&req, 0, sizeof(req));
	req.hdr.nlmsg_type = RTM_GETLINK;
	req.hdr.nlmsg_flags = NLM_F_REQUEST;
	req.hdr.nlmsg_seq = 1;
	req.ifi.ifi_family = AF_UNSPEC;
	req.ifi.ifi_change = ~0;
	req.ahdr.nla_type = IFLA_IFNAME;

	struct iovec iov = { &req, sizeof(req) };
	struct msghdr msg = { &sa, sizeof(sa), &iov, 1, NULL, 0, 0 };

#define TEST_ERROR_SEGMENT(errno)                                          \
	TEST_SUCC(sendmsg(sock_fd, &msg, 0));                              \
	TEST_RES(recv(sock_fd, buffer, BUFFER_SIZE, 0),                    \
		 ((struct nlmsghdr *)buffer)->nlmsg_type == NLMSG_ERROR && \
			 ((struct nlmsgerr *)NLMSG_DATA(buffer))->error == \
				 -errno);

	// ifname = "a" * IFNAMSIZ without nul
	req.hdr.nlmsg_len = sizeof(struct nl_req);
	req.ahdr.nla_len = sizeof(struct nlattr) + IFNAMSIZ;
	memset(req.abuf, 'a', sizeof(req.abuf));
	TEST_ERROR_SEGMENT(ERANGE);

	// ifname = "a" * (IFNAMSIZ - 1) with nul
	req.abuf[IFNAMSIZ - 1] = '\0';
	TEST_ERROR_SEGMENT(ENODEV);

	// ifname = "a" * (IFNAMSIZ - 1) without nul
	req.hdr.nlmsg_len = sizeof(struct nl_req) - 1;
	req.ahdr.nla_len = sizeof(struct nlattr) + IFNAMSIZ - 1;
	req.abuf[IFNAMSIZ - 1] = 'a';
	TEST_ERROR_SEGMENT(ENODEV);

	// ifname = "" without nul
	req.hdr.nlmsg_len = sizeof(struct nl_req) - IFNAMSIZ;
	req.ahdr.nla_len = sizeof(struct nlattr);
	TEST_ERROR_SEGMENT(ERANGE);

	// ifname = "a" without nul
	req.hdr.nlmsg_len = sizeof(struct nl_req) - IFNAMSIZ + 1;
	req.ahdr.nla_len = sizeof(struct nlattr) + 1;
	TEST_ERROR_SEGMENT(ENODEV);

	// ifname = "" with nul
	req.abuf[0] = '\0';
	TEST_ERROR_SEGMENT(ENODEV);

	// Invalid name attribute (too short) without index
	req.hdr.nlmsg_len = sizeof(struct nl_req) - IFNAMSIZ;
	TEST_ERROR_SEGMENT(EINVAL);

	// Invalid name attribute (too short) with index
	req.ifi.ifi_index = 1234;
	// FIXME: Asterinas will report `EINVAL` because it performs strict validation.
#ifdef __asterinas__
	TEST_ERROR_SEGMENT(EINVAL);
#else
	TEST_ERROR_SEGMENT(ENODEV);
#endif

	// Invalid name attribute (too long) with index
	req.hdr.nlmsg_len = sizeof(struct nl_req) + 1;
	req.ahdr.nla_len = sizeof(struct nlattr) + IFNAMSIZ + 1;
	req.abuf[0] = 'a';
	req.abuf[1] = '\0';
	iov.iov_len = req.hdr.nlmsg_len;
	TEST_ERROR_SEGMENT(ERANGE);

	// Invalid message body
	req.hdr.nlmsg_len = NLMSG_LENGTH(2);
	iov.iov_len = req.hdr.nlmsg_len;
	TEST_ERROR_SEGMENT(EINVAL);

	// Invalid message type
	req.hdr.nlmsg_type = 555;
	TEST_ERROR_SEGMENT(EOPNOTSUPP);

	TEST_SUCC(close(sock_fd));
}
END_TEST()

struct nl_req {
	struct nlmsghdr hdr;
	struct ifaddrmsg ifa;
	struct nlattr ahdr;
	char abuf[4];
};

#define INIT_REQ(req)                                               \
	memset(&req, 0, sizeof(req));                               \
	req.hdr.nlmsg_len = NLMSG_LENGTH(sizeof(struct ifaddrmsg)); \
	req.hdr.nlmsg_type = RTM_GETADDR;                           \
	req.hdr.nlmsg_flags = NLM_F_REQUEST;                        \
	req.hdr.nlmsg_seq = 1;                                      \
	req.ifa.ifa_family = AF_UNSPEC;

FN_TEST(get_addr_error)
{
	int sock_fd;
	struct sockaddr_nl sa;

	sock_fd = TEST_SUCC(socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE));

	memset(&sa, 0, sizeof(sa));
	sa.nl_family = AF_NETLINK;

	TEST_SUCC(bind(sock_fd, (struct sockaddr *)&sa, sizeof(sa)));

	// 1. Without NLM_F_DUMP flag
	struct nl_req req;
	INIT_REQ(req);

	struct iovec iov = { &req, req.hdr.nlmsg_len };
	struct msghdr msg = { &sa, sizeof(sa), &iov, 1, NULL, 0, 0 };

	TEST_SUCC(sendmsg(sock_fd, &msg, 0));
	TEST_RES(recv(sock_fd, buffer, BUFFER_SIZE, 0),
		 ((struct nlmsghdr *)buffer)->nlmsg_type == NLMSG_ERROR &&
			 ((struct nlmsgerr *)NLMSG_DATA(buffer))->error ==
				 -EOPNOTSUPP);

	int found_new_addr;
#define TEST_KERNEL_RESPONSE                                                \
	found_new_addr = 0;                                                 \
	while (1) {                                                         \
		size_t recv_len =                                           \
			TEST_SUCC(recv(sock_fd, buffer, BUFFER_SIZE, 0));   \
                                                                            \
		int found_done =                                            \
			TEST_RES(find_new_addr_until_done(buffer, recv_len, \
							  &found_new_addr), \
				 _ret >= 0);                                \
                                                                            \
		if (found_done != 0) {                                      \
			break;                                              \
		}                                                           \
	}

	// 2. Invalid required index
	req.hdr.nlmsg_flags = NLM_F_REQUEST | NLM_F_DUMP | NLM_F_ACK;
	req.ifa.ifa_index = 9999;
	TEST_SUCC(sendmsg(sock_fd, &msg, 0));
	TEST_KERNEL_RESPONSE;

	// 3. Invalid required family
	req.ifa.ifa_family = 255;
	req.ifa.ifa_index = 0;
	TEST_SUCC(sendmsg(sock_fd, &msg, 0));
	TEST_KERNEL_RESPONSE;

	// 4. Unknown attribute
	req.ahdr.nla_type = 0xdeef;
	req.ahdr.nla_len = sizeof(req.ahdr) + sizeof(req.abuf);
	req.hdr.nlmsg_len = sizeof(req);
	iov = (struct iovec){ &req, sizeof(req) };
	TEST_SUCC(sendmsg(sock_fd, &msg, 0));
	TEST_KERNEL_RESPONSE;

	TEST_SUCC(close(sock_fd));
}
END_TEST()

FN_TEST(bufsize_msgsize)
{
	int sock_fd;
	struct nl_req req;

	sock_fd = TEST_SUCC(
		socket(AF_NETLINK, SOCK_RAW | SOCK_NONBLOCK, NETLINK_ROUTE));
	INIT_REQ(req);

	// Send the request
	TEST_RES(send(sock_fd, &req, sizeof(req), 0), _ret == sizeof(req));

	// The buffer size is too short, but it still succeeds
	TEST_SUCC(recv(sock_fd, buffer, 1, 0));

	// The truncated message is now lost
	TEST_ERRNO(recv(sock_fd, buffer, BUFFER_SIZE, 0), EAGAIN);

	TEST_SUCC(close(sock_fd));
}
END_TEST()

int fill_receive_buffer(int sock_fd, const struct nl_req *req)
{
	struct pollfd pfd = { .fd = sock_fd, .events = POLLIN | POLLOUT };
	int i;

	for (i = 0; i < 4096; ++i) {
		if (send(sock_fd, req, sizeof(*req), 0) != sizeof(*req))
			return -1;
		if (poll(&pfd, 1, 0) < 0)
			return -1;
		switch (pfd.revents) {
		case POLLIN | POLLOUT:
			continue;
		case POLLIN | POLLOUT | POLLERR:
			return 0;
		default:
			return -1;
		}
	}

	return -1;
}

FN_TEST(enobufs)
{
	int sock_fd;
	struct nl_req req;

	sock_fd = TEST_SUCC(
		socket(AF_NETLINK, SOCK_RAW | SOCK_NONBLOCK, NETLINK_ROUTE));
	INIT_REQ(req);

	TEST_RES(fill_receive_buffer(sock_fd, &req), _ret >= 0);

	// Now the receive buffer is full. We can still send a new message,
	// but the first `recv` should fail with `ENOBUFS`.
	TEST_RES(send(sock_fd, &req, sizeof(req), 0), _ret == sizeof(req));
	TEST_ERRNO(recv(sock_fd, buffer, 1, 0), ENOBUFS);
	TEST_SUCC(recv(sock_fd, buffer, 1, 0));

	TEST_SUCC(close(sock_fd));
}
END_TEST()
