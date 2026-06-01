// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <arpa/inet.h>
#include <fcntl.h>
#include <linux/netlink.h>
#include <linux/rtnetlink.h>
#include <stdlib.h>
#include <sys/socket.h>
#include <unistd.h>

#include "../common/test.h"

#define PAYLOAD "abcdef"
#define PAYLOAD_LEN 6
#define TCP_SETTLE_USEC 100000
#define SHORT_LEN 3

static ssize_t recvmsg_with_flags(int fd, int flags, char *buf, size_t len,
				  int *msg_flags)
{
	struct iovec iov = { .iov_base = buf, .iov_len = len };
	struct msghdr msg = { .msg_iov = &iov, .msg_iovlen = 1 };
	ssize_t ret = CHECK(recvmsg(fd, &msg, flags));

	*msg_flags = msg.msg_flags;
	return ret;
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

#ifdef __asterinas__
static int buffer_has_byte(const char *buf, size_t len, char byte)
{
	for (size_t i = 0; i < len; i++) {
		if (buf[i] != byte) {
			return 0;
		}
	}

	return 1;
}

FN_TEST(tcp_msg_peek_wrapped_receive_buffer)
{
	int listener;
	int send_fd;
	int recv_fd;
	int status_flags;
	int msg_flags = 0;
	int tcp_recv_buf_len = 0;
	socklen_t optlen = sizeof(tcp_recv_buf_len);
	size_t two_thirds_buf_len;
	char *buf;
	struct sockaddr_in addr = { .sin_family = AF_INET };
	socklen_t addr_len = sizeof(addr);

	listener = TEST_RES(socket(AF_INET, SOCK_STREAM, 0), _ret >= 0);
	send_fd = TEST_RES(socket(AF_INET, SOCK_STREAM, 0), _ret >= 0);

	addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
	TEST_SUCC(bind(listener, (struct sockaddr *)&addr, sizeof(addr)));
	TEST_SUCC(getsockname(listener, (struct sockaddr *)&addr, &addr_len));
	TEST_SUCC(listen(listener, 1));
	TEST_SUCC(connect(send_fd, (struct sockaddr *)&addr, addr_len));
	recv_fd = TEST_RES(accept(listener, NULL, NULL), _ret >= 0);
	status_flags = TEST_RES(fcntl(recv_fd, F_GETFL, 0), _ret >= 0);
	TEST_SUCC(fcntl(recv_fd, F_SETFL, status_flags | O_NONBLOCK));
	TEST_SUCC(close(listener));

	TEST_RES(getsockopt(recv_fd, SOL_SOCKET, SO_RCVBUF, &tcp_recv_buf_len,
			    &optlen),
		 _ret == 0 && tcp_recv_buf_len >= 3);
	two_thirds_buf_len = (tcp_recv_buf_len / 3) * 2;

	buf = TEST_RES(malloc(two_thirds_buf_len), _ret != NULL);
	memset(buf, 'a', two_thirds_buf_len);

	TEST_RES(send(send_fd, buf, two_thirds_buf_len, 0),
		 _ret == (ssize_t)two_thirds_buf_len);
	usleep(TCP_SETTLE_USEC);
	TEST_RES(recv(recv_fd, buf, two_thirds_buf_len, 0),
		 _ret == (ssize_t)two_thirds_buf_len);
	usleep(TCP_SETTLE_USEC);
	TEST_RES(send(send_fd, buf, two_thirds_buf_len, 0),
		 _ret == (ssize_t)two_thirds_buf_len);
	usleep(TCP_SETTLE_USEC);

	memset(buf, 0, two_thirds_buf_len);
	TEST_RES(recvmsg_with_flags(recv_fd, MSG_PEEK, buf, two_thirds_buf_len,
				    &msg_flags),
		 _ret == (ssize_t)two_thirds_buf_len &&
			 (msg_flags & MSG_TRUNC) == 0 &&
			 buffer_has_byte(buf, two_thirds_buf_len, 'a'));

	memset(buf, 0, two_thirds_buf_len);
	TEST_RES(recv(recv_fd, buf, two_thirds_buf_len, 0),
		 _ret == (ssize_t)two_thirds_buf_len &&
			 buffer_has_byte(buf, two_thirds_buf_len, 'a'));
	TEST_ERRNO(recv(recv_fd, buf, two_thirds_buf_len, MSG_DONTWAIT),
		   EAGAIN);

	free(buf);
	TEST_SUCC(close(send_fd));
	TEST_SUCC(close(recv_fd));
}
END_TEST()
#endif

FN_TEST(tcp_msg_peek)
{
	int listener;
	int send_fd;
	int recv_fd;
	int status_flags;
	int msg_flags = 0;
	char buf[PAYLOAD_LEN] = {};
	struct sockaddr_in addr = { .sin_family = AF_INET };
	socklen_t addr_len = sizeof(addr);

	listener = TEST_RES(socket(AF_INET, SOCK_STREAM, 0), _ret >= 0);
	send_fd = TEST_RES(socket(AF_INET, SOCK_STREAM, 0), _ret >= 0);

	addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
	TEST_SUCC(bind(listener, (struct sockaddr *)&addr, sizeof(addr)));
	TEST_SUCC(getsockname(listener, (struct sockaddr *)&addr, &addr_len));
	TEST_SUCC(listen(listener, 1));
	TEST_SUCC(connect(send_fd, (struct sockaddr *)&addr, addr_len));
	recv_fd = TEST_RES(accept(listener, NULL, NULL), _ret >= 0);
	status_flags = TEST_RES(fcntl(recv_fd, F_GETFL, 0), _ret >= 0);
	TEST_SUCC(fcntl(recv_fd, F_SETFL, status_flags | O_NONBLOCK));
	TEST_SUCC(close(listener));

	TEST_RES(send(send_fd, PAYLOAD, PAYLOAD_LEN, 0), _ret == PAYLOAD_LEN);
	TEST_RES(recvmsg_with_flags(recv_fd, MSG_PEEK, buf, SHORT_LEN,
				    &msg_flags),
		 _ret == SHORT_LEN && (msg_flags & MSG_TRUNC) == 0 &&
			 memcmp(buf, PAYLOAD, SHORT_LEN) == 0);

	memset(buf, 0, sizeof(buf));
	TEST_RES(recv(recv_fd, buf, sizeof(buf), 0),
		 _ret == PAYLOAD_LEN && memcmp(buf, PAYLOAD, PAYLOAD_LEN) == 0);

	TEST_SUCC(close(send_fd));
	TEST_SUCC(close(recv_fd));
}
END_TEST()

FN_TEST(tcp_msg_trunc)
{
	int listener;
	int send_fd;
	int recv_fd;
	int status_flags;
	int msg_flags = 0;
	char buf[PAYLOAD_LEN] = {};
	struct sockaddr_in addr = { .sin_family = AF_INET };
	socklen_t addr_len = sizeof(addr);

	listener = TEST_RES(socket(AF_INET, SOCK_STREAM, 0), _ret >= 0);
	send_fd = TEST_RES(socket(AF_INET, SOCK_STREAM, 0), _ret >= 0);

	addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
	TEST_SUCC(bind(listener, (struct sockaddr *)&addr, sizeof(addr)));
	TEST_SUCC(getsockname(listener, (struct sockaddr *)&addr, &addr_len));
	TEST_SUCC(listen(listener, 1));
	TEST_SUCC(connect(send_fd, (struct sockaddr *)&addr, addr_len));
	recv_fd = TEST_RES(accept(listener, NULL, NULL), _ret >= 0);
	status_flags = TEST_RES(fcntl(recv_fd, F_GETFL, 0), _ret >= 0);
	TEST_SUCC(fcntl(recv_fd, F_SETFL, status_flags | O_NONBLOCK));
	TEST_SUCC(close(listener));

	TEST_RES(send(send_fd, PAYLOAD, PAYLOAD_LEN, 0), _ret == PAYLOAD_LEN);
	TEST_RES(recvmsg_with_flags(recv_fd, MSG_TRUNC, buf, SHORT_LEN,
				    &msg_flags),
		 _ret == SHORT_LEN && (msg_flags & MSG_TRUNC) == 0);

	memset(buf, 0, sizeof(buf));
	TEST_RES(recv(recv_fd, buf, sizeof(buf), 0),
		 _ret == PAYLOAD_LEN - SHORT_LEN &&
			 memcmp(buf, PAYLOAD + SHORT_LEN,
				PAYLOAD_LEN - SHORT_LEN) == 0);

	TEST_SUCC(close(send_fd));
	TEST_SUCC(close(recv_fd));
}
END_TEST()

FN_TEST(udp_msg_peek)
{
	int send_fd;
	int recv_fd;
	int msg_flags = 0;
	char buf[PAYLOAD_LEN] = {};
	struct sockaddr_in addr = { .sin_family = AF_INET };
	socklen_t addr_len = sizeof(addr);

	send_fd = TEST_RES(socket(AF_INET, SOCK_DGRAM, 0), _ret >= 0);
	recv_fd = TEST_RES(socket(AF_INET, SOCK_DGRAM | SOCK_NONBLOCK, 0),
			   _ret >= 0);

	addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
	TEST_SUCC(bind(recv_fd, (struct sockaddr *)&addr, sizeof(addr)));
	TEST_SUCC(getsockname(recv_fd, (struct sockaddr *)&addr, &addr_len));
	TEST_SUCC(connect(send_fd, (struct sockaddr *)&addr, addr_len));

	TEST_RES(send(send_fd, PAYLOAD, PAYLOAD_LEN, 0), _ret == PAYLOAD_LEN);
	TEST_RES(recvmsg_with_flags(recv_fd, MSG_PEEK, buf, PAYLOAD_LEN,
				    &msg_flags),
		 _ret == PAYLOAD_LEN && (msg_flags & MSG_TRUNC) == 0 &&
			 memcmp(buf, PAYLOAD, PAYLOAD_LEN) == 0);

	memset(buf, 0, sizeof(buf));
	TEST_RES(recv(recv_fd, buf, sizeof(buf), 0),
		 _ret == PAYLOAD_LEN && memcmp(buf, PAYLOAD, PAYLOAD_LEN) == 0);

	TEST_SUCC(close(send_fd));
	TEST_SUCC(close(recv_fd));
}
END_TEST()

FN_TEST(udp_msg_trunc)
{
	int send_fd;
	int recv_fd;
	int msg_flags = 0;
	char buf[PAYLOAD_LEN] = {};
	struct sockaddr_in addr = { .sin_family = AF_INET };
	socklen_t addr_len = sizeof(addr);

	send_fd = TEST_RES(socket(AF_INET, SOCK_DGRAM, 0), _ret >= 0);
	recv_fd = TEST_RES(socket(AF_INET, SOCK_DGRAM | SOCK_NONBLOCK, 0),
			   _ret >= 0);

	addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
	TEST_SUCC(bind(recv_fd, (struct sockaddr *)&addr, sizeof(addr)));
	TEST_SUCC(getsockname(recv_fd, (struct sockaddr *)&addr, &addr_len));
	TEST_SUCC(connect(send_fd, (struct sockaddr *)&addr, addr_len));

	TEST_RES(send(send_fd, PAYLOAD, PAYLOAD_LEN, 0), _ret == PAYLOAD_LEN);
	TEST_RES(recvmsg_with_flags(recv_fd, MSG_TRUNC, buf, SHORT_LEN,
				    &msg_flags),
		 _ret == PAYLOAD_LEN && (msg_flags & MSG_TRUNC) != 0 &&
			 memcmp(buf, PAYLOAD, SHORT_LEN) == 0);
	TEST_ERRNO(recv(recv_fd, buf, sizeof(buf), MSG_DONTWAIT), EAGAIN);

	TEST_SUCC(close(send_fd));
	TEST_SUCC(close(recv_fd));
}
END_TEST()

FN_TEST(unix_stream_msg_peek)
{
	int fds[2] = { -1, -1 };
	int msg_flags = 0;
	char buf[PAYLOAD_LEN] = {};

	TEST_SUCC(socketpair(AF_UNIX, SOCK_STREAM | SOCK_NONBLOCK, 0, fds));
	TEST_RES(send(fds[0], PAYLOAD, PAYLOAD_LEN, 0), _ret == PAYLOAD_LEN);
	TEST_RES(recvmsg_with_flags(fds[1], MSG_PEEK, buf, SHORT_LEN,
				    &msg_flags),
		 _ret == SHORT_LEN && (msg_flags & MSG_TRUNC) == 0 &&
			 memcmp(buf, PAYLOAD, SHORT_LEN) == 0);

	memset(buf, 0, sizeof(buf));
	TEST_RES(recv(fds[1], buf, sizeof(buf), 0),
		 _ret == PAYLOAD_LEN && memcmp(buf, PAYLOAD, PAYLOAD_LEN) == 0);

	TEST_SUCC(close(fds[0]));
	TEST_SUCC(close(fds[1]));
}
END_TEST()

FN_TEST(unix_stream_msg_trunc)
{
	int fds[2] = { -1, -1 };
	int msg_flags = 0;
	char buf[PAYLOAD_LEN] = {};

	TEST_SUCC(socketpair(AF_UNIX, SOCK_STREAM | SOCK_NONBLOCK, 0, fds));
	TEST_RES(send(fds[0], PAYLOAD, PAYLOAD_LEN, 0), _ret == PAYLOAD_LEN);
	TEST_RES(recvmsg_with_flags(fds[1], MSG_TRUNC, buf, SHORT_LEN,
				    &msg_flags),
		 _ret == SHORT_LEN && (msg_flags & MSG_TRUNC) == 0 &&
			 memcmp(buf, PAYLOAD, SHORT_LEN) == 0);

	memset(buf, 0, sizeof(buf));
	TEST_RES(recv(fds[1], buf, sizeof(buf), 0),
		 _ret == PAYLOAD_LEN - SHORT_LEN &&
			 memcmp(buf, PAYLOAD + SHORT_LEN,
				PAYLOAD_LEN - SHORT_LEN) == 0);

	TEST_SUCC(close(fds[0]));
	TEST_SUCC(close(fds[1]));
}
END_TEST()

FN_TEST(unix_record_msg_peek)
{
	int types[] = { SOCK_SEQPACKET, SOCK_DGRAM, SOCK_RAW };

	for (size_t i = 0; i < sizeof(types) / sizeof(types[0]); i++) {
		int fds[2] = { -1, -1 };
		int msg_flags = 0;
		char buf[PAYLOAD_LEN] = {};

		TEST_SUCC(
			socketpair(AF_UNIX, types[i] | SOCK_NONBLOCK, 0, fds));
		TEST_RES(send(fds[0], PAYLOAD, PAYLOAD_LEN, 0),
			 _ret == PAYLOAD_LEN);
		TEST_RES(recvmsg_with_flags(fds[1], MSG_PEEK, buf, PAYLOAD_LEN,
					    &msg_flags),
			 _ret == PAYLOAD_LEN && (msg_flags & MSG_TRUNC) == 0 &&
				 memcmp(buf, PAYLOAD, PAYLOAD_LEN) == 0);

		memset(buf, 0, sizeof(buf));
		TEST_RES(recv(fds[1], buf, sizeof(buf), 0),
			 _ret == PAYLOAD_LEN &&
				 memcmp(buf, PAYLOAD, PAYLOAD_LEN) == 0);

		TEST_SUCC(close(fds[0]));
		TEST_SUCC(close(fds[1]));
	}
}
END_TEST()

FN_TEST(unix_record_msg_trunc)
{
	int types[] = { SOCK_SEQPACKET, SOCK_DGRAM, SOCK_RAW };

	for (size_t i = 0; i < sizeof(types) / sizeof(types[0]); i++) {
		int fds[2] = { -1, -1 };
		int msg_flags = 0;
		char buf[PAYLOAD_LEN] = {};

		TEST_SUCC(
			socketpair(AF_UNIX, types[i] | SOCK_NONBLOCK, 0, fds));
		TEST_RES(send(fds[0], PAYLOAD, PAYLOAD_LEN, 0),
			 _ret == PAYLOAD_LEN);
		TEST_RES(recvmsg_with_flags(fds[1], MSG_TRUNC, buf, SHORT_LEN,
					    &msg_flags),
			 _ret == PAYLOAD_LEN && (msg_flags & MSG_TRUNC) != 0 &&
				 memcmp(buf, PAYLOAD, SHORT_LEN) == 0);
		TEST_ERRNO(recv(fds[1], buf, sizeof(buf), MSG_DONTWAIT),
			   EAGAIN);

		TEST_SUCC(close(fds[0]));
		TEST_SUCC(close(fds[1]));
	}
}
END_TEST()

FN_TEST(netlink_msg_peek)
{
	int types[] = { SOCK_RAW, SOCK_DGRAM };
	struct sockaddr_nl addr = { .nl_family = AF_NETLINK };

	for (size_t i = 0; i < sizeof(types) / sizeof(types[0]); i++) {
		int fd;
		int msg_flags = 0;
		char peek_buf[8192] = {};
		char recv_buf[8192] = {};
		ssize_t peek_len;

		fd = TEST_RES(socket(AF_NETLINK, types[i] | SOCK_NONBLOCK,
				     NETLINK_ROUTE),
			      _ret >= 0);
		TEST_SUCC(bind(fd, (struct sockaddr *)&addr, sizeof(addr)));
		TEST_RES(send_getlink_request(fd, i + 1), _ret == 0);

		peek_len = TEST_RES(recvmsg_with_flags(fd, MSG_PEEK, peek_buf,
						       sizeof(peek_buf),
						       &msg_flags),
				    _ret > 0 && (msg_flags & MSG_TRUNC) == 0);
		TEST_RES(recv(fd, recv_buf, sizeof(recv_buf), 0),
			 _ret == peek_len &&
				 memcmp(recv_buf, peek_buf, peek_len) == 0);

		TEST_SUCC(close(fd));
	}
}
END_TEST()

FN_TEST(netlink_msg_trunc)
{
	int types[] = { SOCK_RAW, SOCK_DGRAM };
	struct sockaddr_nl addr = { .nl_family = AF_NETLINK };

	for (size_t i = 0; i < sizeof(types) / sizeof(types[0]); i++) {
		int fd;
		int msg_flags = 0;
		char buf[sizeof(struct nlmsghdr)] = {};

		fd = TEST_RES(socket(AF_NETLINK, types[i] | SOCK_NONBLOCK,
				     NETLINK_ROUTE),
			      _ret >= 0);
		TEST_SUCC(bind(fd, (struct sockaddr *)&addr, sizeof(addr)));
		TEST_RES(send_getlink_request(fd, i + 1), _ret == 0);

		TEST_RES(recvmsg_with_flags(fd, MSG_TRUNC, buf, sizeof(buf),
					    &msg_flags),
			 _ret > (ssize_t)sizeof(buf) &&
				 (msg_flags & MSG_TRUNC) != 0);
		TEST_ERRNO(recv(fd, buf, sizeof(buf), MSG_DONTWAIT), EAGAIN);

		TEST_SUCC(close(fd));
	}
}
END_TEST()
