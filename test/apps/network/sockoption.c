// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <unistd.h>
#include "test.h"

int sockfd;
int option;

FN_SETUP(general)
{
	sockfd = CHECK(socket(AF_INET, SOCK_STREAM, 0));
}
END_SETUP()

FN_TEST(buffer_size)
{
	int sendbuf;
	socklen_t sendbuf_len = sizeof(sendbuf);
	TEST_RES(getsockopt(sockfd, SOL_SOCKET, SO_SNDBUF, &sendbuf,
			    &sendbuf_len),
		 sendbuf_len == sizeof(sendbuf));
}
END_TEST()

FN_TEST(socket_error)
{
	int error;
	socklen_t error_len = sizeof(error);
	TEST_RES(getsockopt(sockfd, SOL_SOCKET, SO_ERROR, &error, &error_len),
		 error_len == sizeof(error) && error == 0);
}
END_TEST()

FN_TEST(nagle)
{
	// Disable Nagle algorithm
	option = 1;
	CHECK(setsockopt(sockfd, IPPROTO_TCP, TCP_NODELAY, &option,
			 sizeof(option)));

	// Get new value
	int nagle;
	socklen_t nagle_len = sizeof(nagle);
	TEST_RES(getsockopt(sockfd, IPPROTO_TCP, TCP_NODELAY, &nagle,
			    &nagle_len),
		 nagle == 1);
}
END_TEST()

FN_TEST(reuseaddr)
{
	// Enable reuse address
	option = 1;
	CHECK(setsockopt(sockfd, SOL_SOCKET, SO_REUSEADDR, &option,
			 sizeof(option)));

	// Get new value
	int reuseaddr;
	socklen_t reuseaddr_len = sizeof(reuseaddr);
	TEST_RES(getsockopt(sockfd, SOL_SOCKET, SO_REUSEADDR, &reuseaddr,
			    &reuseaddr_len),
		 reuseaddr == 1);
}
END_TEST()
