// SPDX-License-Identifier: MPL-2.0

#include <net/if.h>
#include <net/if_arp.h>
#include <netlink/route/addr.h>
#include <linux/if_addr.h>
#include <linux/netlink.h>
#include <linux/rtnetlink.h>
#include <stdint.h>
#include <stddef.h>
#include <sys/socket.h>
#include <unistd.h>

#include "netlink_sockoption.h"

#define ETHER_NAME "eth0"
#define LOOPBACK_NAME "lo"
#define BUFFER_SIZE 8192

#define SUCC(expr) ((expr), 0)

char buffer[BUFFER_SIZE];

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

#define TEST_RECV_ERROR(fd, expected_errno)                                \
	do {                                                               \
		struct nlmsghdr *_nlh = (struct nlmsghdr *)buffer;         \
		struct nlmsgerr *_error = NLMSG_DATA(_nlh);                \
		TEST_RES(recv(fd, buffer, BUFFER_SIZE, 0),                 \
			 _ret >= (ssize_t)NLMSG_LENGTH(sizeof(*_error)) && \
				 NLMSG_OK(_nlh, _ret) &&                   \
				 _nlh->nlmsg_len >=                        \
					 NLMSG_LENGTH(sizeof(*_error)) &&  \
				 _nlh->nlmsg_type == NLMSG_ERROR &&        \
				 !(_nlh->nlmsg_flags & NLM_F_MULTI) &&     \
				 _error->error == -(expected_errno));      \
	} while (0)

#define TEST_ERROR_SEGMENT(fd, message, expected_errno) \
	do {                                            \
		TEST_SUCC(sendmsg(fd, message, 0));     \
		TEST_RECV_ERROR(fd, expected_errno);    \
	} while (0)

// Additional conditions can inspect the returned link through _ifi.
#define TEST_RECV_LINK(fd, cond)                                             \
	do {                                                                 \
		struct nlmsghdr *_nlh = (struct nlmsghdr *)buffer;           \
		struct ifinfomsg *_ifi = NLMSG_DATA(_nlh);                   \
		TEST_RES(recv(fd, buffer, BUFFER_SIZE, 0),                   \
			 _ret >= (ssize_t)NLMSG_LENGTH(sizeof(*_ifi)) &&     \
				 NLMSG_OK(_nlh, _ret) &&                     \
				 _nlh->nlmsg_len >=                          \
					 NLMSG_LENGTH(sizeof(*_ifi)) &&      \
				 _nlh->nlmsg_type == RTM_NEWLINK && (cond)); \
	} while (0)

static int recv_nlmsg_done(int sock_fd, int expected_errno)
{
	for (;;) {
		ssize_t recv_len = recv(sock_fd, buffer, BUFFER_SIZE, 0);
		if (recv_len < 0)
			return -1;
		if (recv_len == 0) {
			errno = EPROTO;
			return -1;
		}

		size_t remaining = (size_t)recv_len;
		for (struct nlmsghdr *nlh = (struct nlmsghdr *)buffer;
		     NLMSG_OK(nlh, remaining);
		     nlh = NLMSG_NEXT(nlh, remaining)) {
			if (nlh->nlmsg_type == NLMSG_DONE) {
				if (nlh->nlmsg_len <
					    NLMSG_LENGTH(sizeof(int)) ||
				    *(int *)NLMSG_DATA(nlh) !=
					    -expected_errno) {
					errno = EPROTO;
					return -1;
				}
				return 0;
			}
			if (nlh->nlmsg_type == NLMSG_ERROR) {
				errno = EPROTO;
				return -1;
			}
		}
		if (remaining != 0) {
			errno = EPROTO;
			return -1;
		}
	}
}

/*
 * Asterinas reports failed dump requests with NLMSG_ERROR;
 * Linux reports the error in NLMSG_DONE during the dump.
 */
#ifdef __asterinas__
#define TEST_RECV_DUMP(fd, expected_errno)                    \
	do {                                                  \
		int _expected_errno = (expected_errno);       \
		if (_expected_errno != 0)                     \
			TEST_RECV_ERROR(fd, _expected_errno); \
		else                                          \
			TEST_SUCC(recv_nlmsg_done(fd, 0));    \
	} while (0)
#else
#define TEST_RECV_DUMP(fd, expected_errno) \
	TEST_SUCC(recv_nlmsg_done(fd, expected_errno))
#endif

struct strict_link_request {
	struct nlmsghdr hdr;
	struct ifinfomsg ifi;
	struct nlattr attr;
};

static void init_strict_link_request(struct strict_link_request *request,
				     uint16_t flags)
{
	memset(request, 0, sizeof(*request));
	request->hdr.nlmsg_len = NLMSG_LENGTH(sizeof(request->ifi));
	request->hdr.nlmsg_type = RTM_GETLINK;
	request->hdr.nlmsg_flags = NLM_F_REQUEST | flags;
	request->hdr.nlmsg_seq = 100;
	request->ifi.ifi_family = AF_UNSPEC;
	request->ifi.ifi_index = if_nametoindex(LOOPBACK_NAME);
}

FN_TEST(strict_getlink_requests)
{
	int sock_fd = TEST_SUCC(socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE));
	struct sockaddr_nl sa = { .nl_family = AF_NETLINK };
	TEST_SUCC(bind(sock_fd, (struct sockaddr *)&sa, sizeof(sa)));

	struct strict_link_request request;
	struct iovec iov = { &request, sizeof(request) };
	struct msghdr msg = { &sa, sizeof(sa), &iov, 1, NULL, 0, 0 };

	init_strict_link_request(&request, 0);
	iov.iov_len = request.hdr.nlmsg_len;
	request.ifi.ifi_change = 1;
	TEST_SUCC(sendmsg(sock_fd, &msg, 0));
	TEST_RECV_LINK(sock_fd, 1);

	request.ifi.ifi_change = 0;
	request.attr.nla_len = sizeof(request.attr);
	request.attr.nla_type = 0xdeef;
	request.hdr.nlmsg_len =
		NLMSG_LENGTH(sizeof(request.ifi)) + sizeof(request.attr);
	iov.iov_len = request.hdr.nlmsg_len;
	TEST_SUCC(sendmsg(sock_fd, &msg, 0));
	TEST_RECV_LINK(sock_fd, 1);

	TEST_SETSOCKOPT_INT(sock_fd, NETLINK_GET_STRICT_CHK, 1);
	request.hdr.nlmsg_len = NLMSG_LENGTH(sizeof(request.ifi));
	iov.iov_len = request.hdr.nlmsg_len;
	request.ifi.ifi_change = 1;
	TEST_ERROR_SEGMENT(sock_fd, &msg, EINVAL);

	request.ifi.ifi_change = 0;
	request.attr.nla_len = sizeof(request.attr);
	request.attr.nla_type = IFLA_MASTER;
	request.hdr.nlmsg_len =
		NLMSG_LENGTH(sizeof(request.ifi)) + sizeof(request.attr);
	iov.iov_len = request.hdr.nlmsg_len;
	TEST_SUCC(sendmsg(sock_fd, &msg, 0));
	/*
	 * FIXME: For an empty IFLA_MASTER payload, Linux returns ERANGE;
	 * Asterinas rejects the unsupported attribute with EINVAL in strict mode.
	 */
#ifdef __asterinas__
	TEST_RECV_ERROR(sock_fd, EINVAL);
#else
	TEST_RECV_ERROR(sock_fd, ERANGE);
#endif

	TEST_SETSOCKOPT_INT(sock_fd, NETLINK_GET_STRICT_CHK, 0);
	TEST_SUCC(sendmsg(sock_fd, &msg, 0));
	/*
	 * FIXME: Without strict checking,
	 * Linux still rejects empty IFLA_MASTER with ERANGE;
	 * Asterinas ignores it.
	 */
#ifdef __asterinas__
	TEST_RECV_LINK(sock_fd, 1);
#else
	TEST_RECV_ERROR(sock_fd, ERANGE);
#endif

	request.ifi.ifi_change = 0;
	request.hdr.nlmsg_flags = NLM_F_REQUEST | NLM_F_DUMP;
	TEST_SUCC(sendmsg(sock_fd, &msg, 0));
	TEST_RECV_DUMP(sock_fd, 0);

	TEST_SUCC(close(sock_fd));
}
END_TEST()

FN_TEST(getlink_ignored_body_fields)
{
	int sock_fd = TEST_SUCC(socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE));
	struct sockaddr_nl sa = { .nl_family = AF_NETLINK };
	TEST_SUCC(bind(sock_fd, (struct sockaddr *)&sa, sizeof(sa)));
	unsigned int loopback_index =
		TEST_RES(if_nametoindex(LOOPBACK_NAME), _ret != 0);

	struct strict_link_request request;
	init_strict_link_request(&request, 0);
	struct iovec iov = { &request, request.hdr.nlmsg_len };
	struct msghdr msg = { &sa, sizeof(sa), &iov, 1, NULL, 0, 0 };
	const struct ifinfomsg bodies[] = {
		{ .ifi_type = UINT16_MAX },
		{ .__ifi_pad = 1 },
		{ .ifi_flags = IFF_UP },
		{ .ifi_flags = UINT32_C(1) << 31 },
		{ .ifi_change = UINT32_MAX },
	};

	// Exercise the default mode, strict checking, and switching back.
	const int strict_modes[] = { 0, 1, 0 };
	for (size_t i = 0; i < sizeof(strict_modes) / sizeof(strict_modes[0]);
	     i++) {
		if (i != 0)
			TEST_SETSOCKOPT_INT(sock_fd, NETLINK_GET_STRICT_CHK,
					    strict_modes[i]);

		for (size_t j = 0; j < sizeof(bodies) / sizeof(bodies[0]);
		     j++) {
			request.ifi = bodies[j];
			request.hdr.nlmsg_flags = NLM_F_REQUEST;
			request.ifi.ifi_index = loopback_index;
			TEST_SUCC(sendmsg(sock_fd, &msg, 0));
			if (strict_modes[i]) {
				TEST_RECV_ERROR(sock_fd, EINVAL);
			} else {
				TEST_RECV_LINK(
					sock_fd,
					_ifi->ifi_index ==
							(int)loopback_index &&
						_ifi->ifi_type ==
							ARPHRD_LOOPBACK);
			}

			request.hdr.nlmsg_flags = NLM_F_REQUEST | NLM_F_DUMP;
			request.ifi.ifi_index = 0;
			TEST_SUCC(sendmsg(sock_fd, &msg, 0));
			TEST_RECV_DUMP(sock_fd, strict_modes[i] ? EINVAL : 0);
		}
	}

	TEST_SUCC(close(sock_fd));
}
END_TEST()

FN_TEST(strict_truncated_bodies)
{
	int sock_fd = TEST_SUCC(socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE));
	struct sockaddr_nl sa = { .nl_family = AF_NETLINK };
	TEST_SUCC(bind(sock_fd, (struct sockaddr *)&sa, sizeof(sa)));
	TEST_SETSOCKOPT_INT(sock_fd, NETLINK_GET_STRICT_CHK, 1);

	struct strict_link_request link_request;
	struct iovec iov = { &link_request, 0 };
	struct msghdr msg = { &sa, sizeof(sa), &iov, 1, NULL, 0, 0 };

	// Alignment bytes must not complete a truncated declared body.
	for (int missing = 1; missing <= 3; missing++) {
		init_strict_link_request(&link_request, 0);
		link_request.hdr.nlmsg_len -= missing;
		iov.iov_base = &link_request;
		iov.iov_len = NLMSG_ALIGN(link_request.hdr.nlmsg_len);
		TEST_ERROR_SEGMENT(sock_fd, &msg, EINVAL);
	}

	init_strict_link_request(&link_request, 0);
	iov.iov_base = &link_request;
	iov.iov_len = link_request.hdr.nlmsg_len;
	TEST_SUCC(sendmsg(sock_fd, &msg, 0));
	TEST_RECV_LINK(sock_fd, 1);

	TEST_SUCC(close(sock_fd));
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

struct loopback_addresses {
	int found_ipv4;
	int found_ipv6;
};

void find_loopback_addresses(struct nl_object *obj, void *arg)
{
	struct loopback_addresses *found = arg;
	struct rtnl_addr *addr = (struct rtnl_addr *)obj;
	struct nl_addr *address;
	char buf[INET6_ADDRSTRLEN + sizeof("/128")];

	int family = rtnl_addr_get_family(addr);
	if (family != AF_INET && family != AF_INET6) {
		return;
	}

	address = rtnl_addr_get_local(addr);

	if (family == AF_INET && address) {
		nl_addr2str(address, buf, sizeof(buf));
		if (strcmp(buf, "127.0.0.1/8") == 0) {
			found->found_ipv4 = 1;
		}
	} else if (family == AF_INET6 && address) {
		nl_addr2str(address, buf, sizeof(buf));
		if (strcmp(buf, "::1") == 0 &&
		    rtnl_addr_get_prefixlen(addr) == 128) {
			found->found_ipv6 = 1;
		}
	}
}

FN_TEST(get_loopback_addresses)
{
	struct nl_sock *sock;
	struct nl_cache *addr_cache;

	// 1. Create netlink socket and connect
	sock = nl_socket_alloc();
	TEST_RES(nl_connect(sock, NETLINK_ROUTE), _ret >= 0);

	// 2. Allocate and retrieve address cache
	TEST_RES(rtnl_addr_alloc_cache(sock, &addr_cache), _ret >= 0);

	// 3. Iterate over all addresses to find IPv4 and IPv6 loopback addresses
	struct loopback_addresses found = { 0 };
	TEST_RES(SUCC(nl_cache_foreach(addr_cache, find_loopback_addresses,
				       &found)),
		 found.found_ipv4 && found.found_ipv6);

	// 4. Cleanup
	nl_cache_free(addr_cache);
	nl_close(sock);
	nl_socket_free(sock);
}
END_TEST()

enum reported_address {
	REPORTED_ETHERNET_IPV4 = 1 << 0,
	REPORTED_LOOPBACK_IPV4 = 1 << 1,
	REPORTED_LOOPBACK_IPV6 = 1 << 2,
};

static int validate_reported_address_attributes(struct nlmsghdr *nlh)
{
	struct ifaddrmsg *ifa = NLMSG_DATA(nlh);

	const unsigned char *address = NULL;
	size_t address_len = 0;
	const unsigned char *local = NULL;
	size_t local_len = 0;
	const unsigned char *broadcast = NULL;
	uint32_t flags = 0;
	const char *label = NULL;
	uint8_t protocol = IFAPROT_UNSPEC;
	int attr_len = IFA_PAYLOAD(nlh);
	struct rtattr *attr;

	for (attr = IFA_RTA(ifa); RTA_OK(attr, attr_len);
	     attr = RTA_NEXT(attr, attr_len)) {
		switch (attr->rta_type) {
		case IFA_ADDRESS:
			address = RTA_DATA(attr);
			address_len = RTA_PAYLOAD(attr);
			break;
		case IFA_LOCAL:
			local = RTA_DATA(attr);
			local_len = RTA_PAYLOAD(attr);
			break;
		case IFA_BROADCAST:
			if (RTA_PAYLOAD(attr) != sizeof(struct in_addr))
				return -1;
			broadcast = RTA_DATA(attr);
			break;
		case IFA_FLAGS:
			if (RTA_PAYLOAD(attr) != sizeof(flags))
				return -1;
			memcpy(&flags, RTA_DATA(attr), sizeof(flags));
			break;
		case IFA_LABEL:
			if (!memchr(RTA_DATA(attr), '\0', RTA_PAYLOAD(attr)))
				return -1;
			label = RTA_DATA(attr);
			break;
		case IFA_PROTO:
			if (RTA_PAYLOAD(attr) != sizeof(protocol))
				return -1;
			memcpy(&protocol, RTA_DATA(attr), sizeof(protocol));
			break;
		default:
			break;
		}
	}
	if (attr_len != 0)
		return -1;

	const unsigned char loopback_ipv4[] = { 127, 0, 0, 1 };
	const unsigned char ethernet_ipv4[] = { 10, 0, 2, 15 };
	const unsigned char loopback_ipv6[16] = { [15] = 1 };

	if (ifa->ifa_family == AF_INET &&
	    address_len == sizeof(loopback_ipv4) &&
	    memcmp(address, loopback_ipv4, sizeof(loopback_ipv4)) == 0) {
		if (local_len != sizeof(loopback_ipv4) ||
		    memcmp(local, loopback_ipv4, sizeof(loopback_ipv4)) != 0 ||
		    broadcast != NULL || flags != IFA_F_PERMANENT ||
		    label == NULL || strcmp(label, LOOPBACK_NAME) != 0 ||
		    ifa->ifa_scope != RT_SCOPE_HOST)
			return -1;
		return REPORTED_LOOPBACK_IPV4;
	}

	if (ifa->ifa_family == AF_INET &&
	    address_len == sizeof(ethernet_ipv4) &&
	    memcmp(address, ethernet_ipv4, sizeof(ethernet_ipv4)) == 0) {
		const unsigned char expected_broadcast[] = { 10, 0, 2, 255 };
		if (local_len != sizeof(ethernet_ipv4) ||
		    memcmp(local, ethernet_ipv4, sizeof(ethernet_ipv4)) != 0 ||
		    broadcast == NULL ||
		    memcmp(broadcast, expected_broadcast,
			   sizeof(expected_broadcast)) != 0 ||
		    flags != IFA_F_PERMANENT || label == NULL ||
		    strcmp(label, ETHER_NAME) != 0 ||
		    ifa->ifa_scope != RT_SCOPE_UNIVERSE)
			return -1;
		return REPORTED_ETHERNET_IPV4;
	}

	if (ifa->ifa_family == AF_INET6 &&
	    address_len == sizeof(loopback_ipv6) &&
	    memcmp(address, loopback_ipv6, sizeof(loopback_ipv6)) == 0) {
		if (local != NULL || flags != IFA_F_PERMANENT ||
		    protocol != IFAPROT_KERNEL_LO ||
		    ifa->ifa_scope != RT_SCOPE_HOST)
			return -1;
		return REPORTED_LOOPBACK_IPV6;
	}

	return 0;
}

static int get_reported_addresses(int sock_fd, int family, uint8_t scope)
{
	struct {
		struct nlmsghdr hdr;
		struct ifaddrmsg ifa;
	} req = {
		.hdr = {
			.nlmsg_len = NLMSG_LENGTH(sizeof(struct ifaddrmsg)),
			.nlmsg_type = RTM_GETADDR,
			.nlmsg_flags = NLM_F_REQUEST | NLM_F_DUMP,
			.nlmsg_seq = family + 1,
		},
		.ifa = { .ifa_family = family, .ifa_scope = scope },
	};
	if (send(sock_fd, &req, sizeof(req), 0) != (ssize_t)sizeof(req))
		return -1;

	int found_reports = 0;
	for (;;) {
		ssize_t recv_ret = recv(sock_fd, buffer, BUFFER_SIZE, 0);
		if (recv_ret <= 0)
			return -1;
		size_t recv_len = recv_ret;

		struct nlmsghdr *nlh = (struct nlmsghdr *)buffer;

		for (; NLMSG_OK(nlh, recv_len);
		     nlh = NLMSG_NEXT(nlh, recv_len)) {
			if (nlh->nlmsg_type == NLMSG_DONE)
				return found_reports;
			if (nlh->nlmsg_type != RTM_NEWADDR)
				return -1;
			int report = validate_reported_address_attributes(nlh);
			if (report < 0)
				return -1;
			found_reports |= report;
		}
	}
}

FN_TEST(get_address_attributes)
{
	int sock_fd = TEST_SUCC(socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE));
	struct sockaddr_nl sa = { .nl_family = AF_NETLINK };
	TEST_SUCC(bind(sock_fd, (struct sockaddr *)&sa, sizeof(sa)));

	int expected_ipv4 = REPORTED_LOOPBACK_IPV4;
#ifdef __asterinas__
	expected_ipv4 |= REPORTED_ETHERNET_IPV4;
#endif

	TEST_RES(get_reported_addresses(sock_fd, AF_UNSPEC, 0),
		 _ret == (expected_ipv4 | REPORTED_LOOPBACK_IPV6));
	TEST_RES(get_reported_addresses(sock_fd, AF_INET, 0),
		 _ret == expected_ipv4);
	TEST_RES(get_reported_addresses(sock_fd, AF_INET6, 0),
		 _ret == REPORTED_LOOPBACK_IPV6);

	TEST_SUCC(close(sock_fd));
}
END_TEST()

FN_TEST(legacy_getaddr_ignores_scope)
{
	int sock_fd = TEST_SUCC(socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE));
	struct sockaddr_nl sa = { .nl_family = AF_NETLINK };
	TEST_SUCC(bind(sock_fd, (struct sockaddr *)&sa, sizeof(sa)));

	const int families[] = { AF_INET, AF_INET6 };
	int expected_reports[2];
	for (size_t i = 0; i < sizeof(families) / sizeof(families[0]); i++)
		expected_reports[i] = TEST_RES(
			get_reported_addresses(sock_fd, families[i], 0),
			_ret > 0);

	struct {
		struct nlmsghdr hdr;
		struct ifaddrmsg ifa;
	} request = {
		.hdr = {
			.nlmsg_len = NLMSG_LENGTH(sizeof(struct ifaddrmsg)),
			.nlmsg_type = RTM_GETADDR,
			.nlmsg_flags = NLM_F_REQUEST | NLM_F_DUMP,
			.nlmsg_seq = 100,
		},
		.ifa = { .ifa_scope = 1 },
	};

	// User-defined scopes must be ignored only when strict checking is off.
	const int strict_modes[] = { 0, 1, 0 };
	for (size_t i = 0; i < sizeof(strict_modes) / sizeof(strict_modes[0]);
	     i++) {
		if (i != 0)
			TEST_SETSOCKOPT_INT(sock_fd, NETLINK_GET_STRICT_CHK,
					    strict_modes[i]);

		for (size_t j = 0; j < sizeof(families) / sizeof(families[0]);
		     j++) {
			if (strict_modes[i]) {
				request.ifa.ifa_family = families[j];
				TEST_RES(send(sock_fd, &request,
					      sizeof(request), 0),
					 _ret == (ssize_t)sizeof(request));
				TEST_RECV_DUMP(sock_fd, EINVAL);
			} else {
				TEST_RES(get_reported_addresses(
						 sock_fd, families[j],
						 request.ifa.ifa_scope),
					 _ret == expected_reports[j]);
			}
		}
	}

	TEST_SUCC(close(sock_fd));
}
END_TEST()

FN_TEST(getaddr_reserved_body_fields)
{
	int sock_fd = TEST_SUCC(socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE));
	struct sockaddr_nl sa = { .nl_family = AF_NETLINK };
	TEST_SUCC(bind(sock_fd, (struct sockaddr *)&sa, sizeof(sa)));

	struct {
		struct nlmsghdr hdr;
		struct ifaddrmsg ifa;
	} request = {
		.hdr = {
			.nlmsg_len = NLMSG_LENGTH(sizeof(struct ifaddrmsg)),
			.nlmsg_type = RTM_GETADDR,
			.nlmsg_flags = NLM_F_REQUEST | NLM_F_DUMP,
			.nlmsg_seq = 100,
		},
	};
	const struct {
		struct ifaddrmsg body;
		int strict_errno;
	} cases[] = {
		{ .body = { .ifa_prefixlen = 24 }, .strict_errno = EINVAL },
		{ .body = { .ifa_flags = IFA_F_PERMANENT },
		  .strict_errno = EINVAL },
		{ .body = { .ifa_scope = RT_SCOPE_HOST },
		  .strict_errno = EINVAL },
		{ .body = { 0 }, .strict_errno = 0 },
	};
	const int families[] = { AF_INET, AF_INET6 };
	for (int strict = 0; strict <= 1; strict++) {
		TEST_SETSOCKOPT_INT(sock_fd, NETLINK_GET_STRICT_CHK, strict);
		for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); i++) {
			request.ifa = cases[i].body;
			for (size_t j = 0;
			     j < sizeof(families) / sizeof(families[0]); j++) {
				request.ifa.ifa_family = families[j];
				TEST_RES(send(sock_fd, &request,
					      sizeof(request), 0),
					 _ret == (ssize_t)sizeof(request));
				TEST_RECV_DUMP(sock_fd,
					       strict ? cases[i].strict_errno :
							0);
			}
		}
	}

	TEST_SUCC(close(sock_fd));
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

	// ifname = "a" * IFNAMSIZ without nul
	req.hdr.nlmsg_len = sizeof(struct nl_req);
	req.ahdr.nla_len = sizeof(struct nlattr) + IFNAMSIZ;
	memset(req.abuf, 'a', sizeof(req.abuf));
	TEST_ERROR_SEGMENT(sock_fd, &msg, ERANGE);

	// ifname = "a" * (IFNAMSIZ - 1) with nul
	req.abuf[IFNAMSIZ - 1] = '\0';
	TEST_ERROR_SEGMENT(sock_fd, &msg, ENODEV);

	// ifname = "a" * (IFNAMSIZ - 1) without nul
	req.hdr.nlmsg_len = sizeof(struct nl_req) - 1;
	req.ahdr.nla_len = sizeof(struct nlattr) + IFNAMSIZ - 1;
	req.abuf[IFNAMSIZ - 1] = 'a';
	TEST_ERROR_SEGMENT(sock_fd, &msg, ENODEV);

	// ifname = "" without nul
	req.hdr.nlmsg_len = sizeof(struct nl_req) - IFNAMSIZ;
	req.ahdr.nla_len = sizeof(struct nlattr);
	TEST_ERROR_SEGMENT(sock_fd, &msg, ERANGE);

	// ifname = "a" without nul
	req.hdr.nlmsg_len = sizeof(struct nl_req) - IFNAMSIZ + 1;
	req.ahdr.nla_len = sizeof(struct nlattr) + 1;
	TEST_ERROR_SEGMENT(sock_fd, &msg, ENODEV);

	// ifname = "" with nul
	req.abuf[0] = '\0';
	TEST_ERROR_SEGMENT(sock_fd, &msg, ENODEV);

	// ifname = "\xff\xfe" with nul
	const unsigned char non_utf8_ifname[] = { 0xff, 0xfe, '\0' };
	req.hdr.nlmsg_len =
		offsetof(struct nl_req, abuf) + sizeof(non_utf8_ifname);
	req.ahdr.nla_len = sizeof(struct nlattr) + sizeof(non_utf8_ifname);
	memcpy(req.abuf, non_utf8_ifname, sizeof(non_utf8_ifname));
	TEST_ERROR_SEGMENT(sock_fd, &msg, ENODEV);

	// Invalid name attribute (too short) without index
	req.hdr.nlmsg_len = sizeof(struct nl_req) - IFNAMSIZ;
	TEST_ERROR_SEGMENT(sock_fd, &msg, EINVAL);

	// Invalid name attribute (too short) with index
	req.ifi.ifi_index = 1234;
	TEST_ERROR_SEGMENT(sock_fd, &msg, ENODEV);

	// Invalid name attribute (too long) with index
	req.hdr.nlmsg_len = sizeof(struct nl_req) + 1;
	req.ahdr.nla_len = sizeof(struct nlattr) + IFNAMSIZ + 1;
	req.abuf[0] = 'a';
	req.abuf[1] = '\0';
	iov.iov_len = req.hdr.nlmsg_len;
	TEST_ERROR_SEGMENT(sock_fd, &msg, ERANGE);

	// Invalid message body
	req.hdr.nlmsg_len = NLMSG_LENGTH(2);
	iov.iov_len = req.hdr.nlmsg_len;
	TEST_ERROR_SEGMENT(sock_fd, &msg, EINVAL);

	// Invalid message type
	req.hdr.nlmsg_type = 555;
	TEST_ERROR_SEGMENT(sock_fd, &msg, EOPNOTSUPP);

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
