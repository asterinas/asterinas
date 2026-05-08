// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <arpa/inet.h>
#include <fcntl.h>
#include <linux/netlink.h>
#include <linux/rtnetlink.h>
#include <sys/socket.h>
#include <unistd.h>

#include "../common/test.h"

#define PAYLOAD "abcdef"
#define PAYLOAD_LEN 6
#define SHORT_LEN 3

static int set_nonblocking(int fd)
{
	int flags = fcntl(fd, F_GETFL, 0);

	if (flags < 0) {
		return -1;
	}

	return fcntl(fd, F_SETFL, flags | O_NONBLOCK);
}

static ssize_t recvmsg_with_flags(int fd, int flags, char *buf, size_t len,
				  int *msg_flags)
{
	struct iovec iov = { .iov_base = buf, .iov_len = len };
	struct msghdr msg = { .msg_iov = &iov, .msg_iovlen = 1 };
	ssize_t ret = recvmsg(fd, &msg, flags);

	*msg_flags = msg.msg_flags;
	return ret;
}

static int open_unix_pair(int type, int *send_fd, int *recv_fd)
{
	int fds[2];

	if (socketpair(AF_UNIX, type | SOCK_NONBLOCK, 0, fds) < 0) {
		return -1;
	}

	*send_fd = fds[0];
	*recv_fd = fds[1];
	return 0;
}

static int open_tcp_pair(int *send_fd, int *recv_fd)
{
	int listener = -1;
	int client = -1;
	int server = -1;
	struct sockaddr_in addr = { .sin_family = AF_INET };
	socklen_t addr_len = sizeof(addr);

	listener = socket(AF_INET, SOCK_STREAM, 0);
	client = socket(AF_INET, SOCK_STREAM, 0);
	if (listener < 0 || client < 0) {
		goto fail;
	}

	addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
	if (bind(listener, (struct sockaddr *)&addr, sizeof(addr)) < 0 ||
	    getsockname(listener, (struct sockaddr *)&addr, &addr_len) < 0 ||
	    listen(listener, 1) < 0 ||
	    connect(client, (struct sockaddr *)&addr, addr_len) < 0) {
		goto fail;
	}

	server = accept(listener, NULL, NULL);
	if (server < 0 || set_nonblocking(server) < 0) {
		goto fail;
	}

	close(listener);
	*send_fd = client;
	*recv_fd = server;
	return 0;

fail:
	if (listener >= 0) {
		close(listener);
	}
	if (client >= 0) {
		close(client);
	}
	if (server >= 0) {
		close(server);
	}
	return -1;
}

static int open_udp_pair(int *send_fd, int *recv_fd)
{
	int sender = -1;
	int receiver = -1;
	struct sockaddr_in addr = { .sin_family = AF_INET };
	socklen_t addr_len = sizeof(addr);

	sender = socket(AF_INET, SOCK_DGRAM, 0);
	receiver = socket(AF_INET, SOCK_DGRAM | SOCK_NONBLOCK, 0);
	if (sender < 0 || receiver < 0) {
		goto fail;
	}

	addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
	if (bind(receiver, (struct sockaddr *)&addr, sizeof(addr)) < 0 ||
	    getsockname(receiver, (struct sockaddr *)&addr, &addr_len) < 0 ||
	    connect(sender, (struct sockaddr *)&addr, addr_len) < 0) {
		goto fail;
	}

	*send_fd = sender;
	*recv_fd = receiver;
	return 0;

fail:
	if (sender >= 0) {
		close(sender);
	}
	if (receiver >= 0) {
		close(receiver);
	}
	return -1;
}

static int test_msg_peek_connected(int send_fd, int recv_fd, int is_record)
{
	char buf[PAYLOAD_LEN] = {};
	int msg_flags = 0;
	size_t peek_len = is_record ? PAYLOAD_LEN : SHORT_LEN;
	ssize_t ret;

	if (send(send_fd, PAYLOAD, PAYLOAD_LEN, 0) != PAYLOAD_LEN) {
		return -1;
	}

	ret = recvmsg_with_flags(recv_fd, MSG_PEEK, buf, peek_len, &msg_flags);
	if (ret != (ssize_t)peek_len || (msg_flags & MSG_TRUNC) != 0 ||
	    memcmp(buf, PAYLOAD, peek_len) != 0) {
		return -1;
	}

	memset(buf, 0, sizeof(buf));
	ret = recv(recv_fd, buf, sizeof(buf), 0);
	if (ret != PAYLOAD_LEN || memcmp(buf, PAYLOAD, PAYLOAD_LEN) != 0) {
		return -1;
	}

	return 0;
}

static int run_connected_socket_tests(int (*open_pair)(int *, int *),
				      int is_record)
{
	int send_fd;
	int recv_fd;
	int ret;

	if (open_pair(&send_fd, &recv_fd) < 0) {
		return -1;
	}
	ret = test_msg_peek_connected(send_fd, recv_fd, is_record);
	close(send_fd);
	close(recv_fd);
	return ret;
}

static int open_unix_stream_pair(int *send_fd, int *recv_fd)
{
	return open_unix_pair(SOCK_STREAM, send_fd, recv_fd);
}

static int open_unix_seqpacket_pair(int *send_fd, int *recv_fd)
{
	return open_unix_pair(SOCK_SEQPACKET, send_fd, recv_fd);
}

static int open_unix_datagram_pair(int *send_fd, int *recv_fd)
{
	return open_unix_pair(SOCK_DGRAM, send_fd, recv_fd);
}

static int open_unix_raw_pair(int *send_fd, int *recv_fd)
{
	return open_unix_pair(SOCK_RAW, send_fd, recv_fd);
}

static int open_netlink_route(int type)
{
	int fd = socket(AF_NETLINK, type | SOCK_NONBLOCK, NETLINK_ROUTE);
	struct sockaddr_nl addr = { .nl_family = AF_NETLINK };

	if (fd < 0) {
		return -1;
	}
	if (bind(fd, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
		close(fd);
		return -1;
	}

	return fd;
}

static int send_getlink_request(int fd, unsigned int seq)
{
	struct {
		struct nlmsghdr hdr;
		struct ifinfomsg info;
	} req = {
		.hdr = {
			.nlmsg_len = sizeof(req),
			.nlmsg_type = RTM_GETLINK,
			.nlmsg_flags = NLM_F_REQUEST,
			.nlmsg_seq = seq,
		},
		.info = {
			.ifi_family = AF_UNSPEC,
			.ifi_change = 0xffffffff,
		},
	};

	return send(fd, &req, sizeof(req), 0) == sizeof(req) ? 0 : -1;
}

static int test_netlink_msg_peek(int type)
{
	int fd = open_netlink_route(type);
	char peek_buf[8192] = {};
	char recv_buf[8192] = {};
	int msg_flags = 0;
	ssize_t peek_len;
	ssize_t recv_len;

	if (fd < 0) {
		return -1;
	}
	if (send_getlink_request(fd, 1) < 0) {
		close(fd);
		return -1;
	}

	peek_len = recvmsg_with_flags(fd, MSG_PEEK, peek_buf, sizeof(peek_buf),
				      &msg_flags);
	if (peek_len <= 0 || (msg_flags & MSG_TRUNC) != 0) {
		close(fd);
		return -1;
	}

	recv_len = recv(fd, recv_buf, sizeof(recv_buf), 0);
	close(fd);
	return recv_len == peek_len &&
			       memcmp(recv_buf, peek_buf, peek_len) == 0 ?
		       0 :
		       -1;
}

FN_TEST(msg_peek_trunc)
{
	TEST_RES(run_connected_socket_tests(open_tcp_pair, 0), _ret == 0);
	TEST_RES(run_connected_socket_tests(open_udp_pair, 1), _ret == 0);
	TEST_RES(run_connected_socket_tests(open_unix_stream_pair, 0),
		 _ret == 0);
	TEST_RES(run_connected_socket_tests(open_unix_seqpacket_pair, 1),
		 _ret == 0);
	TEST_RES(run_connected_socket_tests(open_unix_datagram_pair, 1),
		 _ret == 0);
	TEST_RES(run_connected_socket_tests(open_unix_raw_pair, 1), _ret == 0);
	TEST_RES(test_netlink_msg_peek(SOCK_RAW), _ret == 0);
	TEST_RES(test_netlink_msg_peek(SOCK_DGRAM), _ret == 0);
}
END_TEST()
