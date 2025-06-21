// SPDX-License-Identifier: MPL-2.0

#include <netlink/netlink.h>
#include <unistd.h>
#include <sys/socket.h>

#include "../test.h"

static struct sockaddr_nl sk_addr = { .nl_family = AF_NETLINK };

#define C_PORT 1001
#define S_PORT 1002

static int sk_unbound;
static int sk_bound;
static int sk_connected;

FN_SETUP(unbound)
{
	sk_unbound = CHECK(socket(PF_NETLINK, SOCK_DGRAM | SOCK_NONBLOCK,
				  NETLINK_KOBJECT_UEVENT));
}
END_SETUP()

FN_SETUP(bound)
{
	sk_bound = CHECK(socket(PF_NETLINK, SOCK_DGRAM | SOCK_NONBLOCK,
				NETLINK_KOBJECT_UEVENT));

	sk_addr.nl_pid = C_PORT;
	CHECK(bind(sk_bound, (struct sockaddr *)&sk_addr, sizeof(sk_addr)));
}
END_SETUP()

FN_SETUP(connected)
{
	sk_connected = CHECK(
		socket(PF_NETLINK, SOCK_DGRAM | SOCK_NONBLOCK, NETLINK_ROUTE));

	sk_addr.nl_pid = S_PORT;
	CHECK(connect(sk_connected, (struct sockaddr *)&sk_addr,
		      sizeof(sk_addr)));
}
END_SETUP()

FN_TEST(getsockname)
{
	struct sockaddr_nl saddr = { .nl_pid = 0xbeef };
	struct sockaddr *psaddr = (struct sockaddr *)&saddr;
	socklen_t addrlen = 0;

	TEST_RES(getsockname(sk_unbound, psaddr, &addrlen),
		 addrlen == sizeof(saddr) && saddr.nl_pid == 0xbeef);

	TEST_RES(getsockname(sk_unbound, psaddr, &addrlen),
		 addrlen == sizeof(saddr) && saddr.nl_pid == 0);

	TEST_RES(getsockname(sk_bound, psaddr, &addrlen),
		 addrlen == sizeof(saddr) && saddr.nl_pid == C_PORT);

	TEST_RES(getsockname(sk_connected, psaddr, &addrlen),
		 addrlen == sizeof(saddr) && saddr.nl_pid != C_PORT);
}
END_TEST()

FN_TEST(getpeername)
{
	struct sockaddr_nl saddr = { .nl_pid = 0xbeef };
	struct sockaddr *psaddr = (struct sockaddr *)&saddr;
	socklen_t addrlen = sizeof(saddr);

	TEST_RES(getpeername(sk_unbound, psaddr, &addrlen),
		 addrlen == sizeof(saddr) && saddr.nl_pid == 0);

	TEST_RES(getpeername(sk_bound, psaddr, &addrlen),
		 addrlen == sizeof(saddr) && saddr.nl_pid == 0);

	TEST_RES(getpeername(sk_connected, psaddr, &addrlen),
		 addrlen == sizeof(saddr) && saddr.nl_pid == S_PORT);
}
END_TEST()

FN_TEST(send)
{
	char buf[1] = { 'z' };

	TEST_SUCC(send(sk_bound, buf, 1, 0));
	TEST_ERRNO(send(sk_bound, buf, 0, 0), ENODATA);
	TEST_SUCC(write(sk_bound, buf, 1));
	TEST_ERRNO(write(sk_bound, buf, 0), ENODATA);

	TEST_ERRNO(send(sk_connected, buf, 1, 0), ECONNREFUSED);
	TEST_ERRNO(send(sk_connected, buf, 0, 0), ENODATA);
	TEST_ERRNO(write(sk_connected, buf, 1), ECONNREFUSED);
	TEST_ERRNO(write(sk_connected, buf, 0), ENODATA);
}
END_TEST()

FN_TEST(recv)
{
	char buf[1] = { 'z' };

	TEST_ERRNO(recv(sk_unbound, buf, 1, 0), EAGAIN);
	TEST_ERRNO(recv(sk_unbound, buf, 0, 0), EAGAIN);
	TEST_ERRNO(read(sk_unbound, buf, 1), EAGAIN);
	TEST_SUCC(read(sk_unbound, buf, 0));

	TEST_ERRNO(recv(sk_bound, buf, 1, 0), EAGAIN);
	TEST_ERRNO(recv(sk_bound, buf, 0, 0), EAGAIN);
	TEST_ERRNO(read(sk_bound, buf, 1), EAGAIN);
	TEST_SUCC(read(sk_bound, buf, 0));

	TEST_ERRNO(recv(sk_connected, buf, 1, 0), EAGAIN);
	TEST_ERRNO(recv(sk_connected, buf, 0, 0), EAGAIN);
	TEST_ERRNO(read(sk_connected, buf, 1), EAGAIN);
	TEST_SUCC(read(sk_connected, buf, 0));
}
END_TEST()

FN_TEST(bind)
{
	struct sockaddr *psaddr = (struct sockaddr *)&sk_addr;
	socklen_t addrlen = sizeof(sk_addr);

	TEST_ERRNO(bind(sk_unbound, psaddr, addrlen - 1), EINVAL);

	TEST_ERRNO(bind(sk_bound, psaddr, addrlen), EINVAL);

	TEST_ERRNO(bind(sk_connected, psaddr, addrlen), EINVAL);
}
END_TEST()

FN_TEST(listen)
{
	TEST_ERRNO(listen(sk_unbound, 2), EOPNOTSUPP);

	TEST_ERRNO(listen(sk_bound, 2), EOPNOTSUPP);

	TEST_ERRNO(listen(sk_connected, 2), EOPNOTSUPP);
}
END_TEST()

FN_TEST(accept)
{
	struct sockaddr_nl saddr;
	struct sockaddr *psaddr = (struct sockaddr *)&saddr;
	socklen_t addrlen = sizeof(saddr);

	TEST_ERRNO(accept(sk_unbound, psaddr, &addrlen), EOPNOTSUPP);

	TEST_ERRNO(accept(sk_bound, psaddr, &addrlen), EOPNOTSUPP);

	TEST_ERRNO(accept(sk_connected, psaddr, &addrlen), EOPNOTSUPP);
}
END_TEST()

FN_TEST(poll)
{
	struct pollfd pfd = { .events = POLLIN | POLLOUT };

	pfd.fd = sk_unbound;
	TEST_RES(poll(&pfd, 1, 0),
		 (pfd.revents & (POLLIN | POLLOUT)) == POLLOUT);

	pfd.fd = sk_bound;
	TEST_RES(poll(&pfd, 1, 0),
		 (pfd.revents & (POLLIN | POLLOUT)) == POLLOUT);

	pfd.fd = sk_connected;
	TEST_RES(poll(&pfd, 1, 0),
		 (pfd.revents & (POLLIN | POLLOUT)) == POLLOUT);
}
END_TEST()

FN_TEST(connect)
{
	struct sockaddr *psaddr = (struct sockaddr *)&sk_addr;
	socklen_t addrlen = sizeof(sk_addr);

	TEST_SUCC(connect(sk_connected, psaddr, addrlen));
}
END_TEST()

FN_TEST(bind_to_group)
{
	struct sockaddr_nl saddr = { .nl_family = AF_NETLINK };
	struct sockaddr *psaddr = (struct sockaddr *)&saddr;
	socklen_t addrlen = sizeof(saddr);
	int port = 1003;

	int sk_new = TEST_SUCC(socket(PF_NETLINK, SOCK_DGRAM | SOCK_NONBLOCK,
				      NETLINK_KOBJECT_UEVENT));

	saddr.nl_pid = port;
	saddr.nl_groups = 0x1111;
	TEST_SUCC(bind(sk_new, psaddr, addrlen));
	TEST_RES(getsockname(sk_new, psaddr, &addrlen),
		 saddr.nl_pid == port && saddr.nl_groups == 0x1111);

	saddr.nl_groups = 0x111100;
	TEST_SUCC(bind(sk_new, psaddr, addrlen));
	TEST_RES(getsockname(sk_new, psaddr, &addrlen),
		 saddr.nl_pid == port && saddr.nl_groups == 0x111100);

	saddr.nl_groups = -1;
	TEST_SUCC(bind(sk_new, psaddr, addrlen));
	TEST_RES(getsockname(sk_new, psaddr, &addrlen),
		 saddr.nl_pid == port && saddr.nl_groups == 0xffffffff);

	saddr.nl_pid = 2003;
	TEST_ERRNO(bind(sk_new, psaddr, addrlen), EINVAL);

	TEST_SUCC(close(sk_new));
}
END_TEST()

FN_TEST(add_and_drop_membership)
{
	struct sockaddr_nl saddr = { .nl_family = AF_NETLINK,
				     .nl_pid = 0xbeef,
				     .nl_groups = 0xbeef };
	struct sockaddr *psaddr = (struct sockaddr *)&saddr;
	socklen_t addrlen = sizeof(saddr);
	int port = 1004;

	int sk_new = TEST_SUCC(socket(PF_NETLINK, SOCK_DGRAM | SOCK_NONBLOCK,
				      NETLINK_KOBJECT_UEVENT));

	int group = 1;
	TEST_SUCC(setsockopt(sk_new, SOL_NETLINK, NETLINK_ADD_MEMBERSHIP,
			     &group, sizeof(group)));
	TEST_RES(getsockname(sk_new, psaddr, &addrlen),
		 saddr.nl_pid == 0 && saddr.nl_groups == 1);

	char buf[1] = { 'z' };
	TEST_ERRNO(recv(sk_new, buf, 1, 0), EAGAIN);

	saddr.nl_pid = port;
	TEST_SUCC(bind(sk_new, psaddr, addrlen));
	TEST_RES(getsockname(sk_new, psaddr, &addrlen),
		 saddr.nl_pid == port && saddr.nl_groups == 1);

	TEST_SUCC(setsockopt(sk_new, SOL_NETLINK, NETLINK_ADD_MEMBERSHIP,
			     &group, sizeof(group)));
	TEST_RES(getsockname(sk_new, psaddr, &addrlen),
		 saddr.nl_pid == port && saddr.nl_groups == group);

	TEST_SUCC(setsockopt(sk_new, SOL_NETLINK, NETLINK_DROP_MEMBERSHIP,
			     &group, sizeof(group)));
	TEST_RES(getsockname(sk_new, psaddr, &addrlen),
		 saddr.nl_pid == port && saddr.nl_groups == 0);

	group = 2;
	TEST_SUCC(setsockopt(sk_new, SOL_NETLINK, NETLINK_DROP_MEMBERSHIP,
			     &group, sizeof(group)));

	TEST_SUCC(close(sk_new));
}
END_TEST()
