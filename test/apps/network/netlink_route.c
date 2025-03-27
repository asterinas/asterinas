// SPDX-License-Identifier: MPL-2.0

#include <netlink/netlink.h>
#include <netlink/route/link.h>
#include <netlink/route/addr.h>
#include <net/if.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

#include "test.h"

#define ETHER_NAME "eth0"
#define LOOPBACK_NAME "lo"

FN_TEST(if_nameindex)
{
	struct if_nameindex *if_ni, *i;

	if_ni = if_nameindex();
	if (if_ni == NULL) {
		perror("if_nameindex");
		exit(EXIT_FAILURE);
	}

	int found_links = 0;
	for (i = if_ni; !(i->if_index == 0 && i->if_name == NULL); i++) {
		if (strcmp(i->if_name, LOOPBACK_NAME) == 0) {
			found_links++;
		}

		if (strcmp(i->if_name, ETHER_NAME) == 0) {
			found_links++;
		}
	}

	if (found_links != 2) {
		perror("found_links");
		exit(EXIT_FAILURE);
	}

	if_freenameindex(if_ni);
}
END_TEST()

void find_lo_and_eth0(struct nl_object *obj, void *arg)
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

FN_TEST(get_lo_and_eth0_link)
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
	nl_cache_foreach(link_cache, find_lo_and_eth0, &found_links);
	if (found_links != 2) {
		perror("found_links");
		exit(EXIT_FAILURE);
	}

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
	nl_cache_foreach(addr_cache, find_loopback_address, &found_loopback);
	if (found_loopback != 1) {
		perror("found_loopback");
		exit(EXIT_FAILURE);
	}

	// 4. Cleanup
	nl_cache_free(addr_cache);
	nl_close(sock);
	nl_socket_free(sock);
}
END_TEST()
