// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <arpa/inet.h>
#include <sys/socket.h>
#include <unistd.h>

#include "../common/test.h"

#define PAYLOAD "abcdef"
#define PAYLOAD_LEN 6
#define SHORT_LEN 3

static ssize_t recvmsg_with_flags(int fd, int flags, char *buf, size_t len,
				  int *msg_flags)
{
	struct iovec iov = { .iov_base = buf, .iov_len = len };
	struct msghdr msg = { .msg_iov = &iov, .msg_iovlen = 1 };
	ssize_t ret = recvmsg(fd, &msg, flags);

	if (ret >= 0)
		*msg_flags = msg.msg_flags;
	return ret;
}

FN_TEST(tcp_msg_trunc)
{
	int listener;
	int send_fd;
	int recv_fd;
	int msg_flags = 0;
	char buf[PAYLOAD_LEN] = {};
	struct sockaddr_in addr = { .sin_family = AF_INET };
	socklen_t addr_len = sizeof(addr);

	listener = TEST_SUCC(socket(AF_INET, SOCK_STREAM, 0));
	send_fd = TEST_SUCC(socket(AF_INET, SOCK_STREAM, 0));

	addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
	TEST_SUCC(bind(listener, (struct sockaddr *)&addr, sizeof(addr)));
	TEST_SUCC(getsockname(listener, (struct sockaddr *)&addr, &addr_len));
	TEST_SUCC(listen(listener, 1));
	TEST_SUCC(connect(send_fd, (struct sockaddr *)&addr, addr_len));
	recv_fd = TEST_SUCC(accept(listener, NULL, NULL));
	TEST_SUCC(close(listener));

	TEST_RES(send(send_fd, PAYLOAD, PAYLOAD_LEN, 0), _ret == PAYLOAD_LEN);
	memset(buf, 'X', SHORT_LEN);
	TEST_RES(recvmsg_with_flags(recv_fd, MSG_TRUNC, buf, SHORT_LEN,
				    &msg_flags),
		 _ret == SHORT_LEN && (msg_flags & MSG_TRUNC) == 0 &&
			 memcmp(buf, "XXX", SHORT_LEN) == 0);
	memset(buf, 0, sizeof(buf));
	TEST_RES(recv(recv_fd, buf, sizeof(buf), 0),
		 _ret == PAYLOAD_LEN - SHORT_LEN &&
			 memcmp(buf, PAYLOAD + SHORT_LEN,
				PAYLOAD_LEN - SHORT_LEN) == 0);

	TEST_RES(send(send_fd, PAYLOAD, PAYLOAD_LEN, 0), _ret == PAYLOAD_LEN);
	memset(buf, 'X', SHORT_LEN);
	TEST_RES(recvmsg_with_flags(recv_fd, MSG_PEEK | MSG_TRUNC, buf,
				    SHORT_LEN, &msg_flags),
		 _ret == SHORT_LEN && (msg_flags & MSG_TRUNC) == 0 &&
			 memcmp(buf, "XXX", SHORT_LEN) == 0);
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

	send_fd = TEST_SUCC(socket(AF_INET, SOCK_DGRAM, 0));
	recv_fd = TEST_SUCC(socket(AF_INET, SOCK_DGRAM | SOCK_NONBLOCK, 0));

	addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
	TEST_SUCC(bind(recv_fd, (struct sockaddr *)&addr, sizeof(addr)));
	TEST_SUCC(getsockname(recv_fd, (struct sockaddr *)&addr, &addr_len));
	TEST_SUCC(connect(send_fd, (struct sockaddr *)&addr, addr_len));

	TEST_RES(send(send_fd, PAYLOAD, PAYLOAD_LEN, 0), _ret == PAYLOAD_LEN);
	TEST_RES(recvmsg_with_flags(recv_fd, 0, buf, SHORT_LEN, &msg_flags),
		 _ret == SHORT_LEN && (msg_flags & MSG_TRUNC) != 0 &&
			 memcmp(buf, PAYLOAD, SHORT_LEN) == 0);

	TEST_RES(send(send_fd, PAYLOAD, PAYLOAD_LEN, 0), _ret == PAYLOAD_LEN);
	memset(buf, 0, sizeof(buf));
	msg_flags = 0;
	TEST_RES(recvmsg_with_flags(recv_fd, MSG_TRUNC, buf, SHORT_LEN,
				    &msg_flags),
		 _ret == PAYLOAD_LEN && (msg_flags & MSG_TRUNC) != 0 &&
			 memcmp(buf, PAYLOAD, SHORT_LEN) == 0);
	TEST_ERRNO(recv(recv_fd, buf, sizeof(buf), 0), EAGAIN);

	TEST_RES(send(send_fd, PAYLOAD, PAYLOAD_LEN, 0), _ret == PAYLOAD_LEN);
	memset(buf, 0, sizeof(buf));
	msg_flags = 0;
	TEST_RES(recvmsg_with_flags(recv_fd, MSG_PEEK | MSG_TRUNC, buf,
				    SHORT_LEN, &msg_flags),
		 _ret == PAYLOAD_LEN && (msg_flags & MSG_TRUNC) != 0 &&
			 memcmp(buf, PAYLOAD, SHORT_LEN) == 0);
	memset(buf, 0, sizeof(buf));
	TEST_RES(recv(recv_fd, buf, sizeof(buf), 0),
		 _ret == PAYLOAD_LEN && memcmp(buf, PAYLOAD, PAYLOAD_LEN) == 0);

	TEST_SUCC(close(send_fd));
	TEST_SUCC(close(recv_fd));
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
		TEST_ERRNO(recv(fds[1], buf, sizeof(buf), 0), EAGAIN);

		TEST_SUCC(close(fds[0]));
		TEST_SUCC(close(fds[1]));
	}
}
END_TEST()
