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

#define UNIX_STREAM_WAIT_APPENDED_READABLE() \
	TEST_SUCC(usleep(APPEND_SETTLE_USEC))

#define UNIX_STREAM_CONNECT()                                        \
	do {                                                         \
		int fds[2] = { -1, -1 };                             \
		TEST_SUCC(socketpair(AF_UNIX, SOCK_STREAM, 0, fds)); \
		send_fd = fds[0];                                    \
		recv_fd = fds[1];                                    \
	} while (0)

#define SOCKETPAIR_CONNECT(socket_type) \
	TEST_SUCC(socketpair(AF_UNIX, (socket_type), 0, fds))

#define SOCKETPAIR_CLOSE()                \
	do {                              \
		TEST_SUCC(close(fds[0])); \
		TEST_SUCC(close(fds[1])); \
	} while (0)

#define SOCKETPAIR_SEND(offset, len)                         \
	TEST_RES(send(fds[0], PAYLOAD + (offset), (len), 0), \
		 _ret == (ssize_t)(len))

#define SOCKETPAIR_PEEK(offset, len)                                           \
	do {                                                                   \
		memset(buf, 0, sizeof(buf));                                   \
		msg_flags = 0;                                                 \
		TEST_RES(peek_message(fds[1], buf, (len), &msg_flags),         \
			 _ret == (ssize_t)(len) &&                             \
				 (msg_flags & MSG_TRUNC) == 0 &&               \
				 memcmp(buf, PAYLOAD + (offset), (len)) == 0); \
	} while (0)

#define SOCKETPAIR_RECV(offset, len)                                           \
	do {                                                                   \
		memset(buf, 0, sizeof(buf));                                   \
		TEST_RES(recv(fds[1], buf, (len), 0),                          \
			 _ret == (ssize_t)(len) &&                             \
				 memcmp(buf, PAYLOAD + (offset), (len)) == 0); \
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

#define PREFIX unix_stream_
#define CONNECT() UNIX_STREAM_CONNECT()
#define WAIT_APPENDED_READABLE() UNIX_STREAM_WAIT_APPENDED_READABLE()
#include "msg_peek_stream.h"
#undef PREFIX
#undef CONNECT
#undef WAIT_APPENDED_READABLE

FN_TEST(unix_stream_msg_peek_with_passcred)
{
	int fds[2] = { -1, -1 };
	int optval = 1;
	char buf[PAYLOAD_LEN] = {};
	char control[CMSG_SPACE(sizeof(struct ucred))] = {};
	struct iovec iov = { .iov_base = buf, .iov_len = sizeof(buf) };
	struct msghdr msg = {
		.msg_iov = &iov,
		.msg_iovlen = 1,
		.msg_control = control,
		.msg_controllen = sizeof(control),
	};

	TEST_SUCC(socketpair(AF_UNIX, SOCK_STREAM, 0, fds));
	TEST_SUCC(setsockopt(fds[1], SOL_SOCKET, SO_PASSCRED, &optval,
			     sizeof(optval)));

	TEST_RES(send(fds[0], PAYLOAD, SHORT_LEN, 0), _ret == SHORT_LEN);
	TEST_RES(send(fds[0], PAYLOAD + SHORT_LEN, PAYLOAD_LEN - SHORT_LEN, 0),
		 _ret == PAYLOAD_LEN - SHORT_LEN);

	TEST_RES(recvmsg(fds[1], &msg, MSG_PEEK),
		 _ret == PAYLOAD_LEN && (msg.msg_flags & MSG_CTRUNC) == 0 &&
			 memcmp(buf, PAYLOAD, PAYLOAD_LEN) == 0);
	TEST_RES(CMSG_FIRSTHDR(&msg),
		 _ret != NULL && _ret->cmsg_level == SOL_SOCKET &&
			 _ret->cmsg_type == SCM_CREDENTIALS &&
			 _ret->cmsg_len >= CMSG_LEN(sizeof(struct ucred)));

	memset(buf, 0, sizeof(buf));
	TEST_RES(recv(fds[1], buf, sizeof(buf), 0),
		 _ret == PAYLOAD_LEN && memcmp(buf, PAYLOAD, PAYLOAD_LEN) == 0);

	TEST_SUCC(close(fds[0]));
	TEST_SUCC(close(fds[1]));
}
END_TEST()

FN_TEST(unix_seqpacket_msg_peek)
{
	int fds[2] = { -1, -1 };
	int msg_flags = 0;
	char buf[PAYLOAD_LEN * 2] = {};

	SOCKETPAIR_CONNECT(SOCK_SEQPACKET);

	SOCKETPAIR_SEND(0, PAYLOAD_LEN);

	SOCKETPAIR_PEEK(0, PAYLOAD_LEN);

	SOCKETPAIR_RECV(0, PAYLOAD_LEN);

	SOCKETPAIR_SEND(0, PAYLOAD_LEN);
	SOCKETPAIR_SEND(SHORT_LEN, PAYLOAD_LEN - SHORT_LEN);

	memset(buf, 0, sizeof(buf));
	msg_flags = 0;
	TEST_RES(peek_message(fds[1], buf, sizeof(buf), &msg_flags),
		 _ret == PAYLOAD_LEN && (msg_flags & MSG_TRUNC) == 0 &&
			 memcmp(buf, PAYLOAD, PAYLOAD_LEN) == 0);

	SOCKETPAIR_RECV(0, PAYLOAD_LEN);
	SOCKETPAIR_RECV(SHORT_LEN, PAYLOAD_LEN - SHORT_LEN);

	SOCKETPAIR_CLOSE();
}
END_TEST()

FN_TEST(unix_seqpacket_msg_peek_with_scm_rights)
{
	int fds[2] = { -1, -1 };
	int pipe_fds[2] = { -1, -1 };
	int *cdata;
	char buf[1] = {};
	char control[CMSG_SPACE(sizeof(int))] = {};
	struct iovec iov = { .iov_base = buf, .iov_len = 0 };
	struct msghdr msg = {
		.msg_iov = &iov,
		.msg_iovlen = 1,
		.msg_control = control,
		.msg_controllen = sizeof(control),
	};
	struct cmsghdr *chdr = CMSG_FIRSTHDR(&msg);
	int peeked_fd;
	int received_fd;

	TEST_SUCC(socketpair(AF_UNIX, SOCK_SEQPACKET, 0, fds));
	TEST_SUCC(pipe(pipe_fds));

	chdr->cmsg_level = SOL_SOCKET;
	chdr->cmsg_type = SCM_RIGHTS;
	chdr->cmsg_len = CMSG_LEN(sizeof(int));
	cdata = (int *)CMSG_DATA(chdr);
	cdata[0] = pipe_fds[0];
	TEST_RES(sendmsg(fds[0], &msg, 0), _ret == 0);

	memset(control, 0, sizeof(control));
	msg.msg_controllen = sizeof(control);
	TEST_RES(recvmsg(fds[1], &msg, MSG_PEEK),
		 _ret == 0 && (msg.msg_flags & MSG_CTRUNC) == 0 &&
			 (chdr = CMSG_FIRSTHDR(&msg)) &&
			 chdr->cmsg_level == SOL_SOCKET &&
			 chdr->cmsg_type == SCM_RIGHTS &&
			 chdr->cmsg_len >= CMSG_LEN(sizeof(int)));
	cdata = (int *)CMSG_DATA(chdr);
	peeked_fd = cdata[0];

	TEST_RES(write(pipe_fds[1], "p", 1), _ret == 1);
	TEST_RES(read(peeked_fd, buf, 1), _ret == 1 && buf[0] == 'p');

	memset(control, 0, sizeof(control));
	msg.msg_controllen = sizeof(control);
	TEST_RES(recvmsg(fds[1], &msg, 0),
		 _ret == 0 && (msg.msg_flags & MSG_CTRUNC) == 0 &&
			 (chdr = CMSG_FIRSTHDR(&msg)) &&
			 chdr->cmsg_level == SOL_SOCKET &&
			 chdr->cmsg_type == SCM_RIGHTS &&
			 chdr->cmsg_len >= CMSG_LEN(sizeof(int)));
	cdata = (int *)CMSG_DATA(chdr);
	received_fd = cdata[0];

	TEST_RES(write(pipe_fds[1], "r", 1), _ret == 1);
	TEST_RES(read(received_fd, buf, 1), _ret == 1 && buf[0] == 'r');

	TEST_SUCC(close(peeked_fd));
	TEST_SUCC(close(received_fd));
	TEST_SUCC(close(pipe_fds[0]));
	TEST_SUCC(close(pipe_fds[1]));
	TEST_SUCC(close(fds[0]));
	TEST_SUCC(close(fds[1]));
}
END_TEST()

FN_TEST(unix_dgram_msg_peek)
{
	int fds[2] = { -1, -1 };
	int msg_flags = 0;
	char buf[PAYLOAD_LEN] = {};

	SOCKETPAIR_CONNECT(SOCK_DGRAM);

	SOCKETPAIR_SEND(0, PAYLOAD_LEN);

	SOCKETPAIR_PEEK(0, PAYLOAD_LEN);
	SOCKETPAIR_RECV(0, PAYLOAD_LEN);

	SOCKETPAIR_CLOSE();
}
END_TEST()

FN_TEST(unix_raw_msg_peek)
{
	int fds[2] = { -1, -1 };
	int msg_flags = 0;
	char buf[PAYLOAD_LEN] = {};

	SOCKETPAIR_CONNECT(SOCK_RAW);

	SOCKETPAIR_SEND(0, PAYLOAD_LEN);

	SOCKETPAIR_PEEK(0, PAYLOAD_LEN);
	SOCKETPAIR_RECV(0, PAYLOAD_LEN);

	SOCKETPAIR_CLOSE();
}
END_TEST()

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

FN_TEST(netlink_raw_msg_peek)
{
	struct sockaddr_nl addr = { .nl_family = AF_NETLINK };
	int fd;
	int msg_flags = 0;
	char peek_buf[8192] = {};
	char recv_buf[8192] = {};
	ssize_t peek_len;

	fd = TEST_SUCC(socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE));
	TEST_SUCC(bind(fd, (struct sockaddr *)&addr, sizeof(addr)));

	TEST_RES(send_getlink_request(fd, 1), _ret == 0);

	peek_len = TEST_RES(peek_message(fd, peek_buf, sizeof(peek_buf),
					 &msg_flags),
			    _ret > 0 && (msg_flags & MSG_TRUNC) == 0);

	TEST_RES(recv(fd, recv_buf, sizeof(recv_buf), 0),
		 _ret == peek_len && memcmp(recv_buf, peek_buf, peek_len) == 0);

	TEST_SUCC(close(fd));
}
END_TEST()

FN_TEST(netlink_dgram_msg_peek)
{
	struct sockaddr_nl addr = { .nl_family = AF_NETLINK };
	int fd;
	int msg_flags = 0;
	char peek_buf[8192] = {};
	char recv_buf[8192] = {};
	ssize_t peek_len;

	fd = TEST_SUCC(socket(AF_NETLINK, SOCK_DGRAM, NETLINK_ROUTE));
	TEST_SUCC(bind(fd, (struct sockaddr *)&addr, sizeof(addr)));

	TEST_RES(send_getlink_request(fd, 2), _ret == 0);

	peek_len = TEST_RES(peek_message(fd, peek_buf, sizeof(peek_buf),
					 &msg_flags),
			    _ret > 0 && (msg_flags & MSG_TRUNC) == 0);

	TEST_RES(recv(fd, recv_buf, sizeof(recv_buf), 0),
		 _ret == peek_len && memcmp(recv_buf, peek_buf, peek_len) == 0);

	TEST_SUCC(close(fd));
}
END_TEST()
