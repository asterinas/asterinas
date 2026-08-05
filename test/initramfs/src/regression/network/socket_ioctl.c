// SPDX-License-Identifier: MPL-2.0

#include <arpa/inet.h>
#include <linux/if.h>
#include <linux/if_arp.h>
#include <linux/netlink.h>
#include <linux/sockios.h>
#include <linux/vm_sockets.h>
#include <netinet/in.h>
#include <stdbool.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/socket.h>
#include <unistd.h>

#include "../common/test.h"

struct socket_info {
	int domain;
	int type;
	int protocol;
	int fd;
	bool is_ipv4;
};

static struct socket_info sockets[] = {
	{ AF_INET, SOCK_STREAM, 0, -1, true },
	{ AF_INET, SOCK_DGRAM, 0, -1, true },
	{ AF_INET6, SOCK_STREAM, 0, -1, false },
	{ AF_UNIX, SOCK_STREAM, 0, -1, false },
	{ AF_NETLINK, SOCK_RAW, NETLINK_ROUTE, -1, false },
	{ AF_VSOCK, SOCK_STREAM, 0, -1, false },
};

static void init_loopback_ifreq(struct ifreq *ifr)
{
	memset(ifr, 0, sizeof(*ifr));
	strncpy(ifr->ifr_name, "lo", IFNAMSIZ - 1);
}

static int check_interface(const struct ifreq *ifreqs, int ifreqs_len,
			   const char *name)
{
	for (int offset = 0; offset < ifreqs_len;
	     offset += sizeof(struct ifreq)) {
		if (strcmp(ifreqs[offset / sizeof(struct ifreq)].ifr_name,
			   name) == 0)
			return 0;
	}

	errno = ENODEV;
	return -1;
}

FN_SETUP(general)
{
	for (size_t i = 0; i < sizeof(sockets) / sizeof(sockets[0]); i++) {
		struct socket_info *socket_info = &sockets[i];

		socket_info->fd =
			CHECK(socket(socket_info->domain, socket_info->type,
				     socket_info->protocol));
	}
}
END_SETUP()

FN_TEST(general_interface_queries)
{
	for (size_t i = 0; i < sizeof(sockets) / sizeof(sockets[0]); i++) {
		int fd = sockets[i].fd;

		struct ifreq ifr;
		init_loopback_ifreq(&ifr);
		TEST_RES(ioctl(fd, SIOCGIFINDEX, &ifr), ifr.ifr_ifindex > 0);
		int loopback_index = ifr.ifr_ifindex;

		memset(&ifr, 0, sizeof(ifr));
		ifr.ifr_ifindex = loopback_index;
		TEST_RES(ioctl(fd, SIOCGIFNAME, &ifr),
			 strcmp(ifr.ifr_name, "lo") == 0);

		init_loopback_ifreq(&ifr);
		TEST_RES(ioctl(fd, SIOCGIFFLAGS, &ifr),
			 (ifr.ifr_flags &
			  (IFF_UP | IFF_LOOPBACK | IFF_RUNNING)) ==
				 (IFF_UP | IFF_LOOPBACK | IFF_RUNNING));
		TEST_RES(ioctl(fd, SIOCGIFMETRIC, &ifr), ifr.ifr_metric == 0);
		TEST_RES(ioctl(fd, SIOCGIFMTU, &ifr), ifr.ifr_mtu > 0);
		TEST_RES(ioctl(fd, SIOCGIFHWADDR, &ifr),
			 ifr.ifr_hwaddr.sa_family == ARPHRD_LOOPBACK);
		TEST_RES(ioctl(fd, SIOCGIFTXQLEN, &ifr), ifr.ifr_qlen == 1000);

		memset(&ifr, 0, sizeof(ifr));
		strncpy(ifr.ifr_name, "missing", IFNAMSIZ - 1);
		TEST_ERRNO(ioctl(fd, SIOCGIFINDEX, &ifr), ENODEV);
		memset(&ifr, 0, sizeof(ifr));
		ifr.ifr_ifindex = 0;
		TEST_ERRNO(ioctl(fd, SIOCGIFNAME, &ifr), ENODEV);

		memset(&ifr, 'x', sizeof(ifr));
		TEST_ERRNO(ioctl(fd, SIOCGIFINDEX, &ifr), ENODEV);
	}
}
END_TEST()

FN_TEST(interface_conf)
{
	for (size_t i = 0; i < sizeof(sockets) / sizeof(sockets[0]); i++) {
		int fd = sockets[i].fd;

		struct ifconf ifc = { .ifc_len = 0, .ifc_buf = NULL };
		TEST_RES(ioctl(fd, SIOCGIFCONF, &ifc),
			 ifc.ifc_len >= (int)sizeof(struct ifreq));

		int capacity = ifc.ifc_len;
		struct ifreq *ifreqs =
			TEST_RES(calloc(1, capacity), _ret != NULL);
		ifc.ifc_len = capacity;
		ifc.ifc_buf = (char *)ifreqs;
		TEST_RES(ioctl(fd, SIOCGIFCONF, &ifc),
			 ifc.ifc_len >= (int)sizeof(struct ifreq));
		TEST_SUCC(check_interface(ifreqs, ifc.ifc_len, "lo"));
		free(ifreqs);
	}
}
END_TEST()

FN_TEST(ipv4_interface_queries)
{
	for (size_t i = 0; i < sizeof(sockets) / sizeof(sockets[0]); i++) {
		if (!sockets[i].is_ipv4)
			continue;

		int fd = sockets[i].fd;
		struct ifreq ifr;
		init_loopback_ifreq(&ifr);
		TEST_RES(ioctl(fd, SIOCGIFADDR, &ifr),
			 ifr.ifr_addr.sa_family == AF_INET &&
				 ((struct sockaddr_in *)&ifr.ifr_addr)
						 ->sin_addr.s_addr ==
					 htonl(INADDR_LOOPBACK));
		init_loopback_ifreq(&ifr);
		TEST_RES(ioctl(fd, SIOCGIFDSTADDR, &ifr),
			 ifr.ifr_dstaddr.sa_family == AF_INET &&
				 ((struct sockaddr_in *)&ifr.ifr_dstaddr)
						 ->sin_addr.s_addr ==
					 htonl(INADDR_LOOPBACK));
		TEST_RES(ioctl(fd, SIOCGIFBRDADDR, &ifr),
			 ((struct sockaddr_in *)&ifr.ifr_broadaddr)
					 ->sin_addr.s_addr == 0);
		TEST_RES(ioctl(fd, SIOCGIFNETMASK, &ifr),
			 ((struct sockaddr_in *)&ifr.ifr_netmask)
					 ->sin_addr.s_addr ==
				 htonl(0xff000000));
	}
}
END_TEST()

FN_TEST(non_ipv4_interface_queries)
{
	unsigned long ipv4_only_requests[] = {
		SIOCGIFADDR,
		SIOCGIFDSTADDR,
		SIOCGIFBRDADDR,
		SIOCGIFNETMASK,
	};

	for (size_t i = 0; i < sizeof(sockets) / sizeof(sockets[0]); i++) {
		if (sockets[i].is_ipv4)
			continue;

		for (size_t j = 0; j < sizeof(ipv4_only_requests) /
					       sizeof(ipv4_only_requests[0]);
		     j++) {
			struct ifreq ifr;
			init_loopback_ifreq(&ifr);
			TEST_ERRNO(ioctl(sockets[i].fd, ipv4_only_requests[j],
					 &ifr),
				   ENOTTY);
		}
	}
}
END_TEST()

FN_SETUP(cleanup)
{
	for (size_t i = 0; i < sizeof(sockets) / sizeof(sockets[0]); i++)
		CHECK(close(sockets[i].fd));
}
END_SETUP()
