// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <fcntl.h>
#include <stdbool.h>
#include "../common/test.h"

int sk_sender;
int sk_receiver;
struct sockaddr_in sender_addr;
struct sockaddr_in receiver_addr;
socklen_t sockaddr_len = sizeof(struct sockaddr_in);

#define SENDER_PORT 13245
#define RECEIVER_PORT 13246

#define NUM_MESSAGES 3
#define MAX_BUFFER_SIZE 256

void set_nonblocking(int fd)
{
	int flags = CHECK(fcntl(fd, F_GETFL, 0));
	CHECK(fcntl(fd, F_SETFL, flags | O_NONBLOCK));
}

FN_SETUP(create_and_bind)
{
	sk_sender = CHECK(socket(AF_INET, SOCK_DGRAM, 0));
	sk_receiver = CHECK(socket(AF_INET, SOCK_DGRAM, 0));

	memset(&sender_addr, 0, sockaddr_len);
	sender_addr.sin_family = AF_INET;
	sender_addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
	sender_addr.sin_port = htons(SENDER_PORT);
	CHECK(bind(sk_sender, (struct sockaddr *)&sender_addr, sockaddr_len));

	memset(&receiver_addr, 0, sockaddr_len);
	receiver_addr.sin_family = AF_INET;
	receiver_addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
	receiver_addr.sin_port = htons(RECEIVER_PORT);
	CHECK(bind(sk_receiver, (struct sockaddr *)&receiver_addr,
		   sockaddr_len));

	set_nonblocking(sk_sender);
	set_nonblocking(sk_receiver);

	CHECK(connect(sk_sender, (struct sockaddr *)&receiver_addr,
		      sockaddr_len));
}
END_SETUP()

bool check_len(char buffers[][MAX_BUFFER_SIZE], struct mmsghdr *msgs)
{
	for (int i = 0; i < NUM_MESSAGES; ++i) {
		if (i == NUM_MESSAGES - 1) {
			break;
		}

		if (msgs[i].msg_len != strlen(buffers[i])) {
			return false;
		}
	}

	return true;
}

FN_TEST(sendmmsg)
{
	struct mmsghdr msgs[NUM_MESSAGES];
	struct iovec iovs[NUM_MESSAGES];
	char buffers[NUM_MESSAGES][MAX_BUFFER_SIZE];

	for (int i = 0; i < NUM_MESSAGES; ++i) {
		if (i == NUM_MESSAGES - 1) {
			msgs[i].msg_hdr.msg_name = NULL;
			msgs[i].msg_hdr.msg_namelen = 0;
			msgs[i].msg_hdr.msg_iov = NULL;
			msgs[i].msg_hdr.msg_iovlen = 0x100;
			break;
		}

		memset(buffers[i], 0, MAX_BUFFER_SIZE);
		sprintf(buffers[i], "The %d Hello from sender!", i);
		iovs[i].iov_base = buffers[i];
		iovs[i].iov_len = strlen(buffers[i]);

		msgs[i].msg_hdr.msg_iov = &iovs[i];
		msgs[i].msg_hdr.msg_iovlen = 1;
		msgs[i].msg_hdr.msg_name = NULL;
		msgs[i].msg_hdr.msg_namelen = 0;
		msgs[i].msg_hdr.msg_control = NULL;
		msgs[i].msg_hdr.msg_controllen = 0;
		msgs[i].msg_hdr.msg_flags = 0;
		msgs[i].msg_len = 0x12345;
	}

	TEST_RES(sendmmsg(sk_sender, msgs, NUM_MESSAGES, 0),
		 _ret == NUM_MESSAGES - 1 && check_len(buffers, msgs));

	struct msghdr msghdr;
	char recv_buffers[NUM_MESSAGES][MAX_BUFFER_SIZE];
	for (int i = 0; i < NUM_MESSAGES; ++i) {
		if (i == NUM_MESSAGES - 1) {
			TEST_ERRNO(recvmsg(sk_receiver, &msghdr, 0), EAGAIN);
			break;
		}

		memset(recv_buffers[i], 0, MAX_BUFFER_SIZE);

		iovs[i].iov_base = recv_buffers[i];
		iovs[i].iov_len = MAX_BUFFER_SIZE;
		msghdr.msg_name = NULL;
		msghdr.msg_namelen = 0;
		msghdr.msg_iov = &iovs[i];
		msghdr.msg_iovlen = 1;
		msghdr.msg_control = NULL;
		msghdr.msg_controllen = 0;
		TEST_RES(recvmsg(sk_receiver, &msghdr, 0),
			 _ret == strlen(buffers[i]) &&
				 (strcmp(buffers[i], recv_buffers[i]) == 0));
	}
}
END_TEST()