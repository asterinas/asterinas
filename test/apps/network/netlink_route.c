// SPDX-License-Identifier: MPL-2.0

#include <net/if.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/socket.h>
#include <linux/netlink.h>
#include <linux/rtnetlink.h>
#include <arpa/inet.h>
#include <stdbool.h>

#include "test.h"

FN_TEST(if_nameindex)
{
	struct if_nameindex *if_ni, *i;

	if_ni = if_nameindex();
	if (if_ni == NULL) {
		perror("if_nameindex");
		exit(EXIT_FAILURE);
	}

	for (i = if_ni; !(i->if_index == 0 && i->if_name == NULL); i++)
		printf("%u: %s\n", i->if_index, i->if_name);

	if_freenameindex(if_ni);
}
END_TEST()

#define BUFFER_SIZE 4096

FN_TEST(get_inferfaces)
{
	struct nl_req {
		struct nlmsghdr hdr;
		struct rtgenmsg gen;
	};

	int sock_fd;
	struct sockaddr_nl sa;
	char buffer[BUFFER_SIZE];

	// Create a new netlink socket
	sock_fd = TEST_SUCC(socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE));

	memset(&sa, 0, sizeof(sa));
	sa.nl_family = AF_NETLINK;

	// Bind the socket
	TEST_SUCC(bind(sock_fd, (struct sockaddr *)&sa, sizeof(sa)) < 0);

	// Build the request
	struct nl_req req;
	memset(&req, 0, sizeof(req));
	req.hdr.nlmsg_len = NLMSG_LENGTH(sizeof(struct rtgenmsg));
	req.hdr.nlmsg_type = RTM_GETLINK;
	req.hdr.nlmsg_flags = NLM_F_REQUEST | NLM_F_DUMP;
	req.hdr.nlmsg_seq = 1;
	req.gen.rtgen_family = AF_PACKET;

	// Send the request
	struct iovec iov = { &req, req.hdr.nlmsg_len };
	struct msghdr msg = { &sa, sizeof(sa), &iov, 1, NULL, 0, 0 };

	TEST_SUCC(sendmsg(sock_fd, &msg, 0));

	// Get the response
	ssize_t len;
	bool parse_done = false;
	while (1) {
		if (parse_done) {
			break;
		}

		len = TEST_SUCC(recv(sock_fd, buffer, BUFFER_SIZE, 0));

		struct nlmsghdr *nlh = (struct nlmsghdr *)buffer;

		// 解析每个消息
		for (; NLMSG_OK(nlh, len); nlh = NLMSG_NEXT(nlh, len)) {
			if (nlh->nlmsg_type == NLMSG_DONE) {
				parse_done = true;
			}

			if (nlh->nlmsg_type == NLMSG_ERROR) {
				perror("netlink error");
				parse_done = true;
			}

			if (nlh->nlmsg_type == RTM_NEWLINK) {
				struct ifinfomsg *ifi = NLMSG_DATA(nlh);
				struct rtattr *attr = IFLA_RTA(ifi);
				int remaining = nlh->nlmsg_len -
						NLMSG_LENGTH(sizeof(*ifi));

				// Parse each attribute
				for (; RTA_OK(attr, remaining);
				     attr = RTA_NEXT(attr, remaining)) {
					if (attr->rta_type == IFLA_IFNAME) {
						printf("Interface: %s\n",
						       (char *)RTA_DATA(attr));
						break;
					}
				}
			}
		}
	}

	TEST_SUCC(close(sock_fd));
}
END_TEST()

FN_TEST(get_addresses)
{
	struct nl_req {
		struct nlmsghdr hdr;
		struct ifaddrmsg ifa;
	};

	void parse_rtattr(struct rtattr * tb[], int max, struct rtattr *rta,
			  ssize_t len)
	{
		memset(tb, 0, sizeof(struct rtattr *) * (max + 1));
		while (RTA_OK(rta, len)) {
			if (rta->rta_type <= max) {
				tb[rta->rta_type] = rta;
			}
			rta = RTA_NEXT(rta, len);
		}
	}

	struct sockaddr_nl sa;
	int fd;
	char buffer[BUFFER_SIZE];
	struct nlmsghdr *nlh;
	struct nl_req req;

	// Create a new netlink socket
	fd = TEST_SUCC(socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE));

	// Bind the socket
	memset(&sa, 0, sizeof(sa));
	sa.nl_family = AF_NETLINK;
	sa.nl_groups = 0;

	TEST_SUCC(bind(fd, (struct sockaddr *)&sa, sizeof(sa)));

	// Build the request
	memset(&req, 0, sizeof(req));
	req.hdr.nlmsg_len = NLMSG_LENGTH(sizeof(struct ifaddrmsg));
	req.hdr.nlmsg_type = RTM_GETADDR;
	req.hdr.nlmsg_flags = NLM_F_REQUEST | NLM_F_DUMP;
	req.hdr.nlmsg_seq = 1;
	req.ifa.ifa_family = AF_UNSPEC; // Get address of all families

	// Send the request
	TEST_SUCC(send(fd, &req, req.hdr.nlmsg_len, 0));

	// Receive the response
	ssize_t len;
	bool parse_done = false;
	while (1) {
		if (parse_done) {
			break;
		}

		len = TEST_SUCC(recv(fd, buffer, sizeof(buffer), 0));

		for (nlh = (struct nlmsghdr *)buffer; NLMSG_OK(nlh, len);
		     nlh = NLMSG_NEXT(nlh, len)) {
			if (nlh->nlmsg_type == NLMSG_ERROR) {
				parse_done = true;
				break;
			}

			if (nlh->nlmsg_type == NLMSG_DONE) {
				perror("netlink error");
				parse_done = true;
				break;
			}

			if (nlh->nlmsg_type == RTM_NEWADDR) {
				struct ifaddrmsg *ifa =
					(struct ifaddrmsg *)NLMSG_DATA(nlh);
				struct rtattr *tb[IFA_MAX + 1];
				int ifa_len = nlh->nlmsg_len -
					      NLMSG_LENGTH(sizeof(*ifa));
				char ifname[IF_NAMESIZE] = { 0 };
				char addr_str[INET6_ADDRSTRLEN] = { 0 };

				parse_rtattr(tb, IFA_MAX, IFA_RTA(ifa),
					     ifa_len);

				// Get interface name
				if (tb[IFA_LABEL]) {
					char *name =
						(char *)RTA_DATA(tb[IFA_LABEL]);
					strncpy(ifname, name, IF_NAMESIZE - 1);
				}

				// Get interface address
				if (tb[IFA_ADDRESS]) {
					void *addr = RTA_DATA(tb[IFA_ADDRESS]);
					const char *inet_family =
						(ifa->ifa_family == AF_INET) ?
							"IPv4" :
							"IPv6";

					inet_ntop(ifa->ifa_family, addr,
						  addr_str, sizeof(addr_str));
					printf("Interface: %-8s Family: %-4s Address: %s\n",
					       ifname, inet_family, addr_str);
				}
			}
		}
	}

	TEST_SUCC(close(fd));
}
END_TEST()