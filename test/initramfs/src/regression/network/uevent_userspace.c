// SPDX-License-Identifier: MPL-2.0

#include <errno.h>
#include <linux/netlink.h>
#include <stdbool.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

#include "../common/test.h"

#define GROUP_LOW 2U
#define GROUP_HIGH 4U

static const char payload[] = "uevent";
static const char bulk_payload[4096] = "uevent";

static int uevent_socket(unsigned int groups)
{
	struct sockaddr_nl addr = { .nl_family = AF_NETLINK,
				    .nl_groups = groups };
	int fd = socket(AF_NETLINK, SOCK_RAW | SOCK_NONBLOCK,
			NETLINK_KOBJECT_UEVENT);

	if (fd < 0)
		return -1;
	if (bind(fd, (const struct sockaddr *)&addr, sizeof(addr)) < 0) {
		close(fd);
		return -1;
	}
	return fd;
}

static unsigned int port_of(int fd)
{
	struct sockaddr_nl addr;
	socklen_t len = sizeof(addr);

	if (getsockname(fd, (struct sockaddr *)&addr, &len) < 0)
		return 0;
	return addr.nl_pid;
}

static int connect_to(int fd, unsigned int port, unsigned int groups)
{
	struct sockaddr_nl addr = { .nl_family = AF_NETLINK,
				    .nl_pid = port,
				    .nl_groups = groups };

	return connect(fd, (const struct sockaddr *)&addr, sizeof(addr));
}

static ssize_t send_msg(int fd, const void *buf, size_t len, unsigned int port,
			unsigned int groups)
{
	struct sockaddr_nl addr = { .nl_family = AF_NETLINK,
				    .nl_pid = port,
				    .nl_groups = groups };

	return sendto(fd, buf, len, 0, (const struct sockaddr *)&addr,
		      sizeof(addr));
}

static bool recv_payload(int fd, unsigned int port, unsigned int groups)
{
	struct sockaddr_nl addr;
	socklen_t len = sizeof(addr);
	char buf[sizeof(payload) + 1];

	return recvfrom(fd, buf, sizeof(buf), 0, (struct sockaddr *)&addr,
			&len) == (ssize_t)sizeof(payload) &&
	       len == sizeof(addr) && addr.nl_pid == port &&
	       addr.nl_groups == groups &&
	       memcmp(buf, payload, sizeof(payload)) == 0;
}

static ssize_t fill_queue(int fd, int dst, unsigned int port,
			  unsigned int groups)
{
	int rcvbuf = 0;
	socklen_t len = sizeof(rcvbuf);
	ssize_t ret = 0;

	if (getsockopt(dst, SOL_SOCKET, SO_RCVBUF, &rcvbuf, &len) < 0)
		return -1;

	int attempts = rcvbuf / (int)sizeof(bulk_payload) + 2;
	for (int i = 0; i < attempts; i++) {
		ret = send_msg(fd, bulk_payload, sizeof(bulk_payload), port,
			       groups);
		if (ret != (ssize_t)sizeof(bulk_payload))
			break;
	}
	return ret;
}

FN_TEST(unicast)
{
	char buf[sizeof(bulk_payload)];
	int sender = TEST_SUCC(uevent_socket(0));
	int receiver = TEST_SUCC(uevent_socket(0));
	int other = TEST_SUCC(uevent_socket(0));
	unsigned int sender_port = TEST_RES(port_of(sender), _ret != 0);
	unsigned int receiver_port = TEST_RES(port_of(receiver), _ret != 0);
	unsigned int other_port = TEST_RES(port_of(other), _ret != 0);

	TEST_SUCC(send_msg(sender, payload, sizeof(payload), receiver_port, 0));
	TEST_RES(recv_payload(receiver, sender_port, 0), _ret);

	TEST_SUCC(connect_to(receiver, sender_port, 0));
	TEST_SUCC(send(receiver, payload, sizeof(payload), 0));
	TEST_RES(recv_payload(sender, receiver_port, 0), _ret);
	TEST_SUCC(send_msg(sender, payload, sizeof(payload), receiver_port, 0));
	TEST_RES(recv_payload(receiver, sender_port, 0), _ret);
	TEST_ERRNO(send_msg(other, payload, sizeof(payload), receiver_port, 0),
		   ECONNREFUSED);

	TEST_SUCC(close(other));
	TEST_ERRNO(send_msg(sender, payload, sizeof(payload), other_port, 0),
		   ECONNREFUSED);

	int sndbuf = 0;
	socklen_t sndbuf_len = sizeof(sndbuf);
	TEST_RES(getsockopt(sender, SOL_SOCKET, SO_SNDBUF, &sndbuf,
			    &sndbuf_len),
		 _ret == 0 && sndbuf > 0);
	size_t len = (size_t)sndbuf + 1;
	char *oversized = TEST_RES(calloc(len, 1), _ret != NULL);
	if (oversized) {
		TEST_ERRNO(send_msg(sender, oversized, len, receiver_port, 0),
			   EMSGSIZE);
		free(oversized);
	}

	TEST_ERRNO(fill_queue(sender, receiver, receiver_port, 0), EAGAIN);
	TEST_RES(recv(receiver, buf, sizeof(buf), 0),
		 _ret == (ssize_t)sizeof(buf) &&
			 memcmp(buf, bulk_payload, sizeof(buf)) == 0);
	while (recv(receiver, buf, sizeof(buf), 0) > 0)
		;

	TEST_SUCC(connect_to(receiver, 0, 0));
	TEST_ERRNO(send_msg(sender, payload, sizeof(payload), receiver_port, 0),
		   ECONNREFUSED);
	TEST_RES(send_msg(sender, payload, sizeof(payload), 0, 0),
		 _ret == (ssize_t)sizeof(payload));
	TEST_ERRNO(recv(receiver, buf, sizeof(buf), 0), EAGAIN);
	TEST_SUCC(close(receiver));
	TEST_SUCC(close(sender));
}
END_TEST()

FN_TEST(multicast)
{
	char byte;
	int sender = TEST_SUCC(uevent_socket(GROUP_LOW));
	int low = TEST_SUCC(uevent_socket(GROUP_LOW));
	int high = TEST_SUCC(uevent_socket(GROUP_HIGH));
	unsigned int sender_port = TEST_RES(port_of(sender), _ret != 0);
	unsigned int low_port = TEST_RES(port_of(low), _ret != 0);

	TEST_SUCC(send_msg(sender, payload, sizeof(payload), 0, GROUP_LOW));
	TEST_RES(recv_payload(low, sender_port, GROUP_LOW), _ret);
	TEST_ERRNO(recv(sender, &byte, sizeof(byte), 0), EAGAIN);
	TEST_ERRNO(recv(high, &byte, sizeof(byte), 0), EAGAIN);

	TEST_SUCC(send_msg(sender, payload, sizeof(payload), low_port,
			   GROUP_LOW));
	TEST_RES(recv_payload(low, sender_port, GROUP_LOW), _ret);
	TEST_ERRNO(recv(low, &byte, sizeof(byte), 0), EAGAIN);

	TEST_SUCC(send_msg(sender, payload, sizeof(payload), 0,
			   GROUP_LOW | GROUP_HIGH));
	TEST_RES(recv_payload(low, sender_port, GROUP_LOW), _ret);
	TEST_ERRNO(recv(high, &byte, sizeof(byte), 0), EAGAIN);

	TEST_RES(fill_queue(sender, low, 0, GROUP_LOW),
		 _ret == (ssize_t)sizeof(bulk_payload));
	TEST_ERRNO(recv(low, &byte, sizeof(byte), 0), ENOBUFS);
	TEST_SUCC(close(high));
	TEST_SUCC(close(low));
	TEST_SUCC(close(sender));
}
END_TEST()
