// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <arpa/inet.h>
#include <linux/netlink.h>
#include <linux/rtnetlink.h>
#include <sys/socket.h>
#include <unistd.h>

#include "../common/test.h"

#define PAYLOAD "abcdef"
#define PAYLOAD_LEN 6
#define APPEND_SETTLE_USEC 100000
#define SHORT_LEN 3

static int tcp_listener;
static struct sockaddr_in tcp_addr = { .sin_family = AF_INET };
static socklen_t tcp_addr_len = sizeof(tcp_addr);

#define TCP_CONNECT() refresh_connection(&send_fd, &recv_fd)

#define TCP_WAIT_APPENDED_READABLE()                        \
	do {                                                \
		/*                                        \
		 * `poll` cannot wait for the suffix here \
		 * because the peeked prefix keeps the    \
		 * receive side readable.                 \
		 */ \
		TEST_SUCC(usleep(APPEND_SETTLE_USEC));      \
	} while (0)

static ssize_t peek_message(int fd, char *buf, size_t len, int *msg_flags)
{
	struct iovec iov = { .iov_base = buf, .iov_len = len };
	struct msghdr msg = { .msg_iov = &iov, .msg_iovlen = 1 };
	ssize_t ret = recvmsg(fd, &msg, MSG_PEEK);

	if (ret >= 0)
		*msg_flags = msg.msg_flags;
	return ret;
}

FN_SETUP(create_tcp_listener)
{
	tcp_listener = CHECK(socket(AF_INET, SOCK_STREAM, 0));
	tcp_addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);

	CHECK(bind(tcp_listener, (struct sockaddr *)&tcp_addr, tcp_addr_len));
	CHECK(getsockname(tcp_listener, (struct sockaddr *)&tcp_addr,
			  &tcp_addr_len));
	CHECK(listen(tcp_listener, 1));
}
END_SETUP()

static void refresh_connection(int *send_fd, int *recv_fd)
{
	int connected_fd = CHECK(socket(AF_INET, SOCK_STREAM, 0));
	int accepted_fd;

	CHECK(connect(connected_fd, (struct sockaddr *)&tcp_addr,
		      tcp_addr_len));
	accepted_fd = CHECK(accept(tcp_listener, NULL, NULL));

	*send_fd = connected_fd;
	*recv_fd = accepted_fd;
}

#define PREFIX tcp_
#define CONNECT() TCP_CONNECT()
#define WAIT_APPENDED_READABLE() TCP_WAIT_APPENDED_READABLE()
#include "msg_peek_stream.h"
#undef PREFIX
#undef CONNECT
#undef WAIT_APPENDED_READABLE

FN_SETUP(close_tcp_listener)
{
	CHECK(close(tcp_listener));
}
END_SETUP()

FN_TEST(udp_msg_peek)
{
	int send_fd;
	int recv_fd;
	int msg_flags = 0;
	char buf[PAYLOAD_LEN] = {};
	struct sockaddr_in addr = { .sin_family = AF_INET };
	socklen_t addr_len = sizeof(addr);

	send_fd = TEST_SUCC(socket(AF_INET, SOCK_DGRAM, 0));
	recv_fd = TEST_SUCC(socket(AF_INET, SOCK_DGRAM, 0));

	addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
	TEST_SUCC(bind(recv_fd, (struct sockaddr *)&addr, sizeof(addr)));
	TEST_SUCC(getsockname(recv_fd, (struct sockaddr *)&addr, &addr_len));
	TEST_SUCC(connect(send_fd, (struct sockaddr *)&addr, addr_len));

	TEST_RES(send(send_fd, PAYLOAD, PAYLOAD_LEN, 0), _ret == PAYLOAD_LEN);

	// Peeking a datagram must not consume the datagram.
	TEST_RES(peek_message(recv_fd, buf, PAYLOAD_LEN, &msg_flags),
		 _ret == PAYLOAD_LEN && (msg_flags & MSG_TRUNC) == 0 &&
			 memcmp(buf, PAYLOAD, PAYLOAD_LEN) == 0);

	memset(buf, 0, sizeof(buf));
	TEST_RES(recv(recv_fd, buf, sizeof(buf), 0),
		 _ret == PAYLOAD_LEN && memcmp(buf, PAYLOAD, PAYLOAD_LEN) == 0);

	TEST_SUCC(close(send_fd));
	TEST_SUCC(close(recv_fd));
}
END_TEST()
