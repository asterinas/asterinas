// SPDX-License-Identifier: MPL-2.0

#include <net/if.h>
#include <netinet/in.h>
#include <stdint.h>
#include <errno.h>
#include <linux/if_addr.h>
#include <linux/rtnetlink.h>
#include <stddef.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

#include "../common/test.h"

#define ETHER_NAME "eth0"
#define LOOPBACK_NAME "lo"

#ifndef RTM_F_FIB_MATCH
#define RTM_F_FIB_MATCH 0x2000
#endif

struct route_spec {
	const void *dst;
	size_t dst_size;
	uint8_t dst_len;
	const void *gateway;
	size_t gateway_size;
	uint32_t oif;
	uint32_t table;
	uint32_t priority;
	uint32_t flags;
	uint8_t protocol;
	uint8_t scope;
	uint8_t type;
};

struct route_request {
	struct nlmsghdr hdr;
	struct rtmsg rtmsg;
	char attrs[256];
};

#define BUFFER_SIZE 8192
char buffer[BUFFER_SIZE];

static uint32_t ipv4_addr(uint8_t a, uint8_t b, uint8_t c, uint8_t d)
{
	return htonl(((uint32_t)a << 24) | ((uint32_t)b << 16) |
		     ((uint32_t)c << 8) | d);
}

static const uint32_t IPV4_ZERO = 0;
static const struct in6_addr IPV6_LOOPBACK = IN6ADDR_LOOPBACK_INIT;

#define IPV4_SPEC_ADDR(addr) .dst = &(addr), .dst_size = sizeof(addr)
#define IPV4_SPEC_GATEWAY(addr) .gateway = &(addr), .gateway_size = sizeof(addr)
#define IPV6_SPEC_ADDR(addr) .dst = &(addr), .dst_size = sizeof(addr)
#define NO_GATEWAY .gateway = NULL, .gateway_size = 0

static uint32_t iface_index_by_name(const char *name)
{
	struct if_nameindex *ifaces = CHECK_WITH(if_nameindex(), _ret != NULL);
	uint32_t index = 0;

	for (struct if_nameindex *iface = ifaces;
	     !(iface->if_index == 0 && iface->if_name == NULL); iface++) {
		if (strcmp(iface->if_name, name) == 0) {
			index = iface->if_index;
			break;
		}
	}

	if_freenameindex(ifaces);
	return index;
}

static void init_route_request(struct route_request *req, uint16_t type,
			       uint16_t flags, uint32_t seq)
{
	memset(req, 0, sizeof(*req));
	req->hdr.nlmsg_len = NLMSG_LENGTH(sizeof(req->rtmsg));
	req->hdr.nlmsg_type = type;
	req->hdr.nlmsg_flags = NLM_F_REQUEST | flags;
	req->hdr.nlmsg_seq = seq;
	req->rtmsg.rtm_family = AF_INET;
	req->rtmsg.rtm_protocol = RTPROT_UNSPEC;
	req->rtmsg.rtm_scope = RT_SCOPE_UNIVERSE;
	req->rtmsg.rtm_type = RTN_UNICAST;
}

static int route_matches(struct nlmsghdr *nlh, const struct route_spec *spec)
{
	struct rtmsg *rtmsg = NLMSG_DATA(nlh);
	struct rtattr *rta = RTM_RTA(rtmsg);
	int attr_len = RTM_PAYLOAD(nlh);
	const void *dst = NULL;
	size_t dst_size = 0;
	const void *gateway = NULL;
	size_t gateway_size = 0;
	uint32_t oif = 0;
	uint32_t table = rtmsg->rtm_table;
	uint32_t priority = 0;
	int family = spec->dst_size == sizeof(struct in6_addr) ? AF_INET6 :
								 AF_INET;

	if (nlh->nlmsg_type != RTM_NEWROUTE || rtmsg->rtm_family != family ||
	    rtmsg->rtm_dst_len != spec->dst_len ||
	    rtmsg->rtm_protocol != spec->protocol ||
	    rtmsg->rtm_scope != spec->scope || rtmsg->rtm_type != spec->type ||
	    rtmsg->rtm_flags != spec->flags) {
		return 0;
	}

	for (; RTA_OK(rta, attr_len); rta = RTA_NEXT(rta, attr_len)) {
		switch (rta->rta_type) {
		case RTA_DST:
			dst = RTA_DATA(rta);
			dst_size = RTA_PAYLOAD(rta);
			break;
		case RTA_GATEWAY:
			gateway = RTA_DATA(rta);
			gateway_size = RTA_PAYLOAD(rta);
			break;
		case RTA_OIF:
			if (RTA_PAYLOAD(rta) != sizeof(oif)) {
				break;
			}
			memcpy(&oif, RTA_DATA(rta), sizeof(oif));
			break;
		case RTA_PRIORITY:
			if (RTA_PAYLOAD(rta) != sizeof(priority)) {
				break;
			}
			memcpy(&priority, RTA_DATA(rta), sizeof(priority));
			break;
		case RTA_TABLE:
			if (RTA_PAYLOAD(rta) != sizeof(table)) {
				break;
			}
			memcpy(&table, RTA_DATA(rta), sizeof(table));
			break;
		default:
			break;
		}
	}

	return (dst != NULL) == (spec->dst_len != 0) &&
	       (spec->dst_len == 0 ||
		(dst_size == spec->dst_size &&
		 memcmp(dst, spec->dst, spec->dst_size) == 0)) &&
	       (gateway != NULL) == (spec->gateway_size != 0) &&
	       (spec->gateway_size == 0 ||
		(gateway_size == spec->gateway_size &&
		 memcmp(gateway, spec->gateway, spec->gateway_size) == 0)) &&
	       oif == spec->oif && (spec->table == 0 || table == spec->table) &&
	       priority == spec->priority;
}

static int recv_until_done_or_ack(int sock_fd, uint32_t seq, int dump_request,
				  const struct route_spec *spec,
				  int *found_route, int *done)
{
	while (1) {
		ssize_t ret = CHECK(recv(sock_fd, buffer, BUFFER_SIZE, 0));
		size_t recv_len = ret;
		struct nlmsghdr *nlh = (struct nlmsghdr *)buffer;

		for (; NLMSG_OK(nlh, recv_len);
		     nlh = NLMSG_NEXT(nlh, recv_len)) {
			if (nlh->nlmsg_seq != seq) {
				return -1;
			}
			if (nlh->nlmsg_type == NLMSG_ERROR) {
				struct nlmsgerr *err = NLMSG_DATA(nlh);
				if (err->error != 0) {
					return -1;
				}
				*done = 1;
				return 0;
			}
			if (nlh->nlmsg_type == NLMSG_DONE) {
				*done = 1;
				return 0;
			}
			if (spec != NULL && route_matches(nlh, spec)) {
				*found_route += 1;
				if (!dump_request) {
					*done = 1;
					return 0;
				}
			}
			if (spec != NULL && !dump_request &&
			    nlh->nlmsg_type == RTM_NEWROUTE) {
				*done = 1;
				return 0;
			}
		}
	}
}

static int route_request_success(int sock_fd, const struct route_request *req,
				 const struct route_spec *spec)
{
	struct sockaddr_nl sa = { .nl_family = AF_NETLINK };
	struct iovec iov = { (void *)req, req->hdr.nlmsg_len };
	struct msghdr msg = { &sa, sizeof(sa), &iov, 1, NULL, 0, 0 };
	int found_route = 0;
	int done = 0;

	CHECK_WITH(sendmsg(sock_fd, &msg, 0),
		   _ret == (ssize_t)req->hdr.nlmsg_len);
	CHECK(recv_until_done_or_ack(sock_fd, req->hdr.nlmsg_seq,
				     req->hdr.nlmsg_flags & NLM_F_DUMP, spec,
				     &found_route, &done));

	return done && (spec == NULL || found_route > 0) ? 0 : -1;
}

FN_TEST(get_route_dump_bootstrap)
{
	int sock_fd;
	uint32_t lo_index = iface_index_by_name(LOOPBACK_NAME);
	uint32_t eth0_index = iface_index_by_name(ETHER_NAME);
	uint32_t loopback_net = ipv4_addr(127, 0, 0, 0);
	uint32_t eth0_net = ipv4_addr(10, 0, 2, 0);
	uint32_t gateway = ipv4_addr(10, 0, 2, 2);
	uint32_t eth0_broadcast_addr = ipv4_addr(10, 0, 2, 255);
	uint32_t limited_broadcast_addr = ipv4_addr(255, 255, 255, 255);
	struct route_request req;
	struct route_spec loopback_connected = {
		IPV4_SPEC_ADDR(loopback_net),
		.dst_len = 8,
		NO_GATEWAY,
		.oif = lo_index,
		.table = RT_TABLE_MAIN,
		.protocol = RTPROT_KERNEL,
		.scope = RT_SCOPE_LINK,
		.type = RTN_UNICAST,
	};
	struct route_spec loopback_local = {
		IPV4_SPEC_ADDR(loopback_net),
		.dst_len = 8,
		NO_GATEWAY,
		.oif = lo_index,
		.table = RT_TABLE_LOCAL,
		.protocol = RTPROT_KERNEL,
		.scope = RT_SCOPE_HOST,
		.type = RTN_LOCAL,
	};
	struct route_spec eth0_connected = {
		IPV4_SPEC_ADDR(eth0_net),
		.dst_len = 24,
		NO_GATEWAY,
		.oif = eth0_index,
		.table = RT_TABLE_MAIN,
		.protocol = RTPROT_KERNEL,
		.scope = RT_SCOPE_LINK,
		.type = RTN_UNICAST,
	};
	struct route_spec default_route = {
		IPV4_SPEC_ADDR(IPV4_ZERO),  .dst_len = 0,
		IPV4_SPEC_GATEWAY(gateway), .oif = eth0_index,
		.table = RT_TABLE_MAIN,	    .protocol = RTPROT_BOOT,
		.scope = RT_SCOPE_UNIVERSE, .type = RTN_UNICAST,
	};
	struct route_spec eth0_broadcast = {
		IPV4_SPEC_ADDR(eth0_broadcast_addr),
		.dst_len = 32,
		NO_GATEWAY,
		.oif = eth0_index,
		.table = RT_TABLE_LOCAL,
		.protocol = RTPROT_KERNEL,
		.scope = RT_SCOPE_LINK,
		.type = RTN_BROADCAST,
	};
	struct route_spec limited_broadcast = {
		IPV4_SPEC_ADDR(limited_broadcast_addr),
		.dst_len = 32,
		NO_GATEWAY,
		.oif = eth0_index,
		.table = RT_TABLE_LOCAL,
		.protocol = RTPROT_KERNEL,
		.scope = RT_SCOPE_LINK,
		.type = RTN_BROADCAST,
	};
	struct route_spec loopback_ipv6_connected = {
		IPV6_SPEC_ADDR(IPV6_LOOPBACK),
		.dst_len = 128,
		NO_GATEWAY,
		.oif = lo_index,
		.table = RT_TABLE_MAIN,
		.protocol = RTPROT_KERNEL,
		.scope = RT_SCOPE_LINK,
		.type = RTN_UNICAST,
	};
	struct route_spec loopback_ipv6_local = {
		IPV6_SPEC_ADDR(IPV6_LOOPBACK),
		.dst_len = 128,
		NO_GATEWAY,
		.oif = lo_index,
		.table = RT_TABLE_LOCAL,
		.protocol = RTPROT_KERNEL,
		.scope = RT_SCOPE_HOST,
		.type = RTN_LOCAL,
	};

	sock_fd = TEST_SUCC(socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE));
	init_route_request(&req, RTM_GETROUTE, NLM_F_DUMP, 10);
	req.rtmsg.rtm_protocol = RTPROT_UNSPEC;
	req.rtmsg.rtm_type = RTN_UNSPEC;
	req.rtmsg.rtm_scope = RT_SCOPE_UNIVERSE;
	TEST_RES(route_request_success(sock_fd, &req, &loopback_connected),
		 _ret == 0);
	TEST_RES(route_request_success(sock_fd, &req, &loopback_local),
		 _ret == 0);
	if (eth0_index != 0) {
		TEST_RES(route_request_success(sock_fd, &req, &eth0_connected),
			 _ret == 0);
		TEST_RES(route_request_success(sock_fd, &req, &default_route),
			 _ret == 0);
		TEST_RES(route_request_success(sock_fd, &req,
					       &limited_broadcast),
			 _ret == 0);
		TEST_RES(route_request_success(sock_fd, &req, &eth0_broadcast),
			 _ret == 0);
	}

	init_route_request(&req, RTM_GETROUTE, NLM_F_DUMP, 11);
	req.hdr.nlmsg_len = NLMSG_LENGTH(sizeof(struct rtgenmsg));
	((struct rtgenmsg *)NLMSG_DATA(&req.hdr))->rtgen_family = AF_UNSPEC;
	TEST_RES(route_request_success(sock_fd, &req, &loopback_connected),
		 _ret == 0);
	TEST_RES(route_request_success(sock_fd, &req, &loopback_local),
		 _ret == 0);
	TEST_RES(route_request_success(sock_fd, &req, &loopback_ipv6_connected),
		 _ret == 0);

	init_route_request(&req, RTM_GETROUTE, NLM_F_DUMP, 13);
	req.rtmsg.rtm_family = AF_INET6;
	req.rtmsg.rtm_protocol = RTPROT_UNSPEC;
	req.rtmsg.rtm_type = RTN_UNSPEC;
	req.rtmsg.rtm_scope = RT_SCOPE_UNIVERSE;
	TEST_RES(route_request_success(sock_fd, &req, &loopback_ipv6_connected),
		 _ret == 0);
	TEST_RES(route_request_success(sock_fd, &req, &loopback_ipv6_local),
		 _ret == 0);

	init_route_request(&req, RTM_GETROUTE, NLM_F_DUMP, 12);
	req.rtmsg.rtm_protocol = RTPROT_UNSPEC;
	req.rtmsg.rtm_type = RTN_UNSPEC;
	req.rtmsg.rtm_scope = RT_SCOPE_UNIVERSE;
	req.rtmsg.rtm_flags = RTM_F_CLONED;
	{
		struct sockaddr_nl sa = { .nl_family = AF_NETLINK };
		struct iovec iov = { &req, req.hdr.nlmsg_len };
		struct msghdr msg = { &sa, sizeof(sa), &iov, 1, NULL, 0, 0 };
		int found_routes = 0;
		int done = 0;

		CHECK_WITH(sendmsg(sock_fd, &msg, 0),
			   _ret == (ssize_t)req.hdr.nlmsg_len);
		while (!done) {
			ssize_t ret =
				CHECK(recv(sock_fd, buffer, BUFFER_SIZE, 0));
			size_t recv_len = ret;
			struct nlmsghdr *nlh = (struct nlmsghdr *)buffer;

			for (; NLMSG_OK(nlh, recv_len);
			     nlh = NLMSG_NEXT(nlh, recv_len)) {
				CHECK_WITH(nlh->nlmsg_seq,
					   _ret == req.hdr.nlmsg_seq);
				CHECK_WITH(nlh->nlmsg_type,
					   _ret != NLMSG_ERROR);
				if (nlh->nlmsg_type == NLMSG_DONE) {
					done = 1;
					break;
				}
				CHECK_WITH(nlh->nlmsg_type,
					   _ret == RTM_NEWROUTE);
				found_routes++;
			}
		}
		TEST_RES(found_routes, _ret == 0);
	}

	TEST_SUCC(close(sock_fd));
}
END_TEST()

static int find_iface_ipv4_addr(struct nlmsghdr *nlh, uint32_t iface_index,
				uint32_t *ipv4_addr)
{
	struct ifaddrmsg *ifa = NLMSG_DATA(nlh);
	struct rtattr *rta = IFA_RTA(ifa);
	int attr_len = IFA_PAYLOAD(nlh);
	uint32_t fallback_addr = 0;

	if (nlh->nlmsg_type != RTM_NEWADDR || ifa->ifa_family != AF_INET ||
	    ifa->ifa_index != iface_index) {
		return 0;
	}

	for (; RTA_OK(rta, attr_len); rta = RTA_NEXT(rta, attr_len)) {
		if (RTA_PAYLOAD(rta) != sizeof(*ipv4_addr)) {
			continue;
		}
		if (rta->rta_type == IFA_LOCAL) {
			memcpy(ipv4_addr, RTA_DATA(rta), sizeof(*ipv4_addr));
			return 1;
		}
		if (rta->rta_type == IFA_ADDRESS) {
			memcpy(&fallback_addr, RTA_DATA(rta),
			       sizeof(fallback_addr));
		}
	}

	if (fallback_addr != 0) {
		*ipv4_addr = fallback_addr;
		return 1;
	}

	return 0;
}

static uint32_t iface_ipv4_addr_by_index(uint32_t iface_index)
{
	int sock_fd = CHECK(socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE));
	struct sockaddr_nl sa = { .nl_family = AF_NETLINK };
	struct {
		struct nlmsghdr hdr;
		struct ifaddrmsg ifa;
	} req;
	struct iovec iov = { &req, sizeof(req) };
	struct msghdr msg = { &sa, sizeof(sa), &iov, 1, NULL, 0, 0 };
	uint32_t ipv4_addr = 0;

	memset(&req, 0, sizeof(req));
	req.hdr.nlmsg_len = NLMSG_LENGTH(sizeof(req.ifa));
	req.hdr.nlmsg_type = RTM_GETADDR;
	req.hdr.nlmsg_flags = NLM_F_REQUEST | NLM_F_DUMP;
	req.hdr.nlmsg_seq = 40;
	req.ifa.ifa_family = AF_INET;

	CHECK_WITH(sendmsg(sock_fd, &msg, 0),
		   _ret == (ssize_t)req.hdr.nlmsg_len);
	while (ipv4_addr == 0) {
		ssize_t ret = CHECK(recv(sock_fd, buffer, BUFFER_SIZE, 0));
		size_t recv_len = ret;
		struct nlmsghdr *nlh = (struct nlmsghdr *)buffer;

		for (; NLMSG_OK(nlh, recv_len);
		     nlh = NLMSG_NEXT(nlh, recv_len)) {
			if (nlh->nlmsg_seq != req.hdr.nlmsg_seq) {
				continue;
			}
			if (nlh->nlmsg_type == NLMSG_ERROR) {
				struct nlmsgerr *err = NLMSG_DATA(nlh);

				CHECK_WITH(err->error, _ret == 0);
				CHECK(close(sock_fd));
				return ipv4_addr;
			}
			if (nlh->nlmsg_type == NLMSG_DONE) {
				CHECK(close(sock_fd));
				return ipv4_addr;
			}
			if (find_iface_ipv4_addr(nlh, iface_index,
						 &ipv4_addr)) {
				break;
			}
		}
	}

	CHECK(close(sock_fd));
	return ipv4_addr;
}

static void add_rtattr(struct nlmsghdr *nlh, size_t max_len, uint16_t type,
		       const void *data, size_t data_len)
{
	size_t attr_len = RTA_LENGTH(data_len);
	size_t new_len = NLMSG_ALIGN(nlh->nlmsg_len) + RTA_ALIGN(attr_len);
	struct rtattr *rta;

	if (new_len > max_len) {
		abort();
	}

	rta = (struct rtattr *)((char *)nlh + NLMSG_ALIGN(nlh->nlmsg_len));
	rta->rta_type = type;
	rta->rta_len = attr_len;
	if (data_len != 0) {
		memcpy(RTA_DATA(rta), data, data_len);
	}
	memset((char *)rta + attr_len, 0, RTA_ALIGN(attr_len) - attr_len);
	nlh->nlmsg_len = new_len;
}

static int route_lookup_table_info(int sock_fd, const struct route_request *req,
				   uint32_t *header_table, int *attr_present,
				   uint32_t *attr_table)
{
	struct sockaddr_nl sa = { .nl_family = AF_NETLINK };
	struct iovec iov = { (void *)req, req->hdr.nlmsg_len };
	struct msghdr msg = { &sa, sizeof(sa), &iov, 1, NULL, 0, 0 };

	*header_table = RT_TABLE_UNSPEC;
	*attr_present = 0;
	*attr_table = RT_TABLE_UNSPEC;

	CHECK_WITH(sendmsg(sock_fd, &msg, 0),
		   _ret == (ssize_t)req->hdr.nlmsg_len);
	CHECK(recv(sock_fd, buffer, BUFFER_SIZE, 0));

	struct nlmsghdr *nlh = (struct nlmsghdr *)buffer;
	if (nlh->nlmsg_seq != req->hdr.nlmsg_seq ||
	    nlh->nlmsg_type != RTM_NEWROUTE) {
		return -1;
	}

	struct rtmsg *rtmsg = NLMSG_DATA(nlh);
	*header_table = rtmsg->rtm_table;
	struct rtattr *rta = RTM_RTA(rtmsg);
	int attr_len = RTM_PAYLOAD(nlh);
	for (; RTA_OK(rta, attr_len); rta = RTA_NEXT(rta, attr_len)) {
		if (rta->rta_type == RTA_TABLE &&
		    RTA_PAYLOAD(rta) == sizeof(uint32_t)) {
			memcpy(attr_table, RTA_DATA(rta), sizeof(*attr_table));
			*attr_present = 1;
		}
	}

	return 0;
}

static int route_lookup_prefsrc(int sock_fd, const struct route_request *req,
				void *prefsrc, size_t prefsrc_size)
{
	struct sockaddr_nl sa = { .nl_family = AF_NETLINK };
	struct iovec iov = { (void *)req, req->hdr.nlmsg_len };
	struct msghdr msg = { &sa, sizeof(sa), &iov, 1, NULL, 0, 0 };

	memset(prefsrc, 0, prefsrc_size);

	CHECK_WITH(sendmsg(sock_fd, &msg, 0),
		   _ret == (ssize_t)req->hdr.nlmsg_len);
	CHECK(recv(sock_fd, buffer, BUFFER_SIZE, 0));

	struct nlmsghdr *nlh = (struct nlmsghdr *)buffer;
	if (nlh->nlmsg_seq != req->hdr.nlmsg_seq ||
	    nlh->nlmsg_type != RTM_NEWROUTE) {
		return -1;
	}

	struct rtmsg *rtmsg = NLMSG_DATA(nlh);
	struct rtattr *rta = RTM_RTA(rtmsg);
	int attr_len = RTM_PAYLOAD(nlh);
	for (; RTA_OK(rta, attr_len); rta = RTA_NEXT(rta, attr_len)) {
		if (rta->rta_type == RTA_PREFSRC &&
		    RTA_PAYLOAD(rta) == prefsrc_size) {
			memcpy(prefsrc, RTA_DATA(rta), prefsrc_size);
			return 0;
		}
	}

	return -1;
}

FN_TEST(route_lookup)
{
	int sock_fd;
	uint32_t eth0_index = iface_index_by_name(ETHER_NAME);
	uint32_t eth0_addr;
	struct route_request req;
	uint32_t dst = ipv4_addr(8, 8, 8, 8);
	uint32_t gateway = ipv4_addr(10, 0, 2, 2);
	uint32_t header_table = RT_TABLE_UNSPEC;
	int attr_table_present = 0;
	uint32_t attr_table = RT_TABLE_UNSPEC;
	uint32_t prefsrc = 0;
	struct route_spec lookup_route = {
		IPV4_SPEC_ADDR(dst),	    .dst_len = 32,
		IPV4_SPEC_GATEWAY(gateway), .oif = eth0_index,
		.table = RT_TABLE_UNSPEC,   .flags = RTM_F_CLONED,
		.protocol = RTPROT_UNSPEC,  .scope = RT_SCOPE_UNIVERSE,
		.type = RTN_UNICAST,
	};
	struct route_spec fibmatch_route = {
		IPV4_SPEC_ADDR(IPV4_ZERO),  .dst_len = 0,
		IPV4_SPEC_GATEWAY(gateway), .oif = eth0_index,
		.table = RT_TABLE_MAIN,	    .protocol = RTPROT_BOOT,
		.scope = RT_SCOPE_UNIVERSE, .type = RTN_UNICAST,
	};

	SKIP_TEST_IF(eth0_index == 0);
	eth0_addr = iface_ipv4_addr_by_index(eth0_index);
	SKIP_TEST_IF(eth0_addr == 0);

	sock_fd = TEST_SUCC(socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE));
	init_route_request(&req, RTM_GETROUTE, 0, 50);
	req.rtmsg.rtm_dst_len = 32;
	add_rtattr(&req.hdr, sizeof(req), RTA_DST, &dst, sizeof(dst));

	TEST_RES(route_request_success(sock_fd, &req, &lookup_route),
		 _ret == 0);
	TEST_RES(route_lookup_table_info(sock_fd, &req, &header_table,
					 &attr_table_present, &attr_table),
		 _ret == 0 && header_table == RT_TABLE_MAIN &&
			 attr_table_present == 0);
	TEST_RES(route_lookup_prefsrc(sock_fd, &req, &prefsrc, sizeof(prefsrc)),
		 _ret == 0 && prefsrc == eth0_addr);

	init_route_request(&req, RTM_GETROUTE, 0, 51);
	req.rtmsg.rtm_dst_len = 32;
	req.rtmsg.rtm_flags = RTM_F_LOOKUP_TABLE;
	add_rtattr(&req.hdr, sizeof(req), RTA_DST, &dst, sizeof(dst));
	TEST_RES(route_lookup_table_info(sock_fd, &req, &header_table,
					 &attr_table_present, &attr_table),
		 _ret == 0 && header_table == RT_TABLE_MAIN &&
			 attr_table_present && attr_table == RT_TABLE_MAIN);

	init_route_request(&req, RTM_GETROUTE, 0, 52);
	req.rtmsg.rtm_dst_len = 32;
	req.rtmsg.rtm_flags = RTM_F_FIB_MATCH;
	add_rtattr(&req.hdr, sizeof(req), RTA_DST, &dst, sizeof(dst));
	TEST_RES(route_request_success(sock_fd, &req, &fibmatch_route),
		 _ret == 0);

	TEST_SUCC(close(sock_fd));
}
END_TEST()

FN_TEST(route_lookup_ipv6)
{
	int sock_fd;
	uint32_t lo_index = iface_index_by_name(LOOPBACK_NAME);
	struct route_request req;
	uint32_t header_table = RT_TABLE_UNSPEC;
	int attr_table_present = 0;
	uint32_t attr_table = RT_TABLE_UNSPEC;
	struct in6_addr prefsrc;
	struct route_spec lookup_route = {
		IPV6_SPEC_ADDR(IPV6_LOOPBACK),
		.dst_len = 128,
		NO_GATEWAY,
		.oif = lo_index,
		.table = RT_TABLE_UNSPEC,
		.flags = RTM_F_CLONED,
		.protocol = RTPROT_UNSPEC,
		.scope = RT_SCOPE_HOST,
		.type = RTN_LOCAL,
	};
	struct route_spec fibmatch_route = {
		IPV6_SPEC_ADDR(IPV6_LOOPBACK),
		.dst_len = 128,
		NO_GATEWAY,
		.oif = lo_index,
		.table = RT_TABLE_LOCAL,
		.protocol = RTPROT_KERNEL,
		.scope = RT_SCOPE_HOST,
		.type = RTN_LOCAL,
	};

	SKIP_TEST_IF(lo_index == 0);

	sock_fd = TEST_SUCC(socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE));
	init_route_request(&req, RTM_GETROUTE, 0, 60);
	req.rtmsg.rtm_family = AF_INET6;
	req.rtmsg.rtm_dst_len = 128;
	add_rtattr(&req.hdr, sizeof(req), RTA_DST, &IPV6_LOOPBACK,
		   sizeof(IPV6_LOOPBACK));

	TEST_RES(route_request_success(sock_fd, &req, &lookup_route),
		 _ret == 0);
	TEST_RES(route_lookup_table_info(sock_fd, &req, &header_table,
					 &attr_table_present, &attr_table),
		 _ret == 0 && header_table == RT_TABLE_MAIN &&
			 attr_table_present == 0);
	TEST_RES(route_lookup_prefsrc(sock_fd, &req, &prefsrc, sizeof(prefsrc)),
		 _ret == 0 && memcmp(&prefsrc, &IPV6_LOOPBACK,
				     sizeof(prefsrc)) == 0);

	init_route_request(&req, RTM_GETROUTE, 0, 61);
	req.rtmsg.rtm_family = AF_INET6;
	req.rtmsg.rtm_dst_len = 128;
	req.rtmsg.rtm_flags = RTM_F_LOOKUP_TABLE;
	add_rtattr(&req.hdr, sizeof(req), RTA_DST, &IPV6_LOOPBACK,
		   sizeof(IPV6_LOOPBACK));
	TEST_RES(route_lookup_table_info(sock_fd, &req, &header_table,
					 &attr_table_present, &attr_table),
		 _ret == 0 && header_table == RT_TABLE_LOCAL &&
			 attr_table_present && attr_table == RT_TABLE_LOCAL);

	init_route_request(&req, RTM_GETROUTE, 0, 62);
	req.rtmsg.rtm_family = AF_INET6;
	req.rtmsg.rtm_dst_len = 128;
	req.rtmsg.rtm_flags = RTM_F_FIB_MATCH;
	add_rtattr(&req.hdr, sizeof(req), RTA_DST, &IPV6_LOOPBACK,
		   sizeof(IPV6_LOOPBACK));
	TEST_RES(route_request_success(sock_fd, &req, &fibmatch_route),
		 _ret == 0);

	TEST_SUCC(close(sock_fd));
}
END_TEST()

static int route_request_error(int sock_fd, const struct route_request *req,
			       int expected_errno)
{
	struct sockaddr_nl sa = { .nl_family = AF_NETLINK };
	struct iovec iov = { (void *)req, req->hdr.nlmsg_len };
	struct msghdr msg = { &sa, sizeof(sa), &iov, 1, NULL, 0, 0 };

	CHECK_WITH(sendmsg(sock_fd, &msg, 0),
		   _ret == (ssize_t)req->hdr.nlmsg_len);
	CHECK(recv(sock_fd, buffer, BUFFER_SIZE, 0));

	struct nlmsghdr *nlh = (struct nlmsghdr *)buffer;
	if (nlh->nlmsg_seq != req->hdr.nlmsg_seq ||
	    nlh->nlmsg_type != NLMSG_ERROR) {
		return -1;
	}

	CHECK_WITH(-((struct nlmsgerr *)NLMSG_DATA(nlh))->error,
		   _ret == expected_errno);
	return 0;
}

FN_TEST(route_query_error)
{
	int sock_fd;
	uint32_t eth0_index = iface_index_by_name(ETHER_NAME);
	struct route_request req;
	uint32_t dst = ipv4_addr(8, 8, 8, 8);
	uint32_t gateway = ipv4_addr(10, 0, 2, 2);
	struct route_spec default_lookup = {
		IPV4_SPEC_ADDR(dst),	    .dst_len = 32,
		IPV4_SPEC_GATEWAY(gateway), .oif = eth0_index,
		.table = RT_TABLE_UNSPEC,   .flags = RTM_F_CLONED,
		.protocol = RTPROT_UNSPEC,  .scope = RT_SCOPE_UNIVERSE,
		.type = RTN_UNICAST,
	};

	SKIP_TEST_IF(eth0_index == 0);

	sock_fd = TEST_SUCC(socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE));

	init_route_request(&req, RTM_GETROUTE, 0, 99);
	req.rtmsg.rtm_dst_len = 32;
	req.rtmsg.rtm_src_len = 32;
	add_rtattr(&req.hdr, sizeof(req), RTA_DST, &dst, sizeof(dst));
	TEST_RES(route_request_error(sock_fd, &req, EOPNOTSUPP), _ret == 0);

	init_route_request(&req, RTM_NEWROUTE, NLM_F_CREATE | NLM_F_ACK, 100);
	TEST_RES(route_request_error(sock_fd, &req, EOPNOTSUPP), _ret == 0);

	init_route_request(&req, RTM_DELROUTE, NLM_F_ACK, 101);
	TEST_RES(route_request_error(sock_fd, &req, EOPNOTSUPP), _ret == 0);

	init_route_request(&req, RTM_GETROUTE, 0, 102);
	req.rtmsg.rtm_dst_len = 32;
	add_rtattr(&req.hdr, sizeof(req), RTA_DST, &dst, sizeof(dst));
	TEST_RES(route_request_success(sock_fd, &req, &default_lookup),
		 _ret == 0);

	TEST_SUCC(close(sock_fd));
}
END_TEST()
