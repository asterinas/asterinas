// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <unistd.h>
#include <sys/signal.h>
#include <sys/socket.h>
#include <sys/poll.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <fcntl.h>
#include <stddef.h>

#include "test.h"

#define S_PORT htons(0x1238)

struct sockaddr_in sk_addr;
struct pollfd pfd = { .events = POLLIN | POLLOUT | POLLRDHUP };
char buf[4096] = { 'a' };

int sk_listen;
int sk_connect;
int sk_accept;

FN_TEST(poll_unconnected)
{
	sk_listen = CHECK(socket(PF_INET, SOCK_STREAM, 0));

	pfd.fd = sk_listen;
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLOUT | POLLHUP));

	sk_addr.sin_family = AF_INET;
	sk_addr.sin_port = S_PORT;
	CHECK(inet_aton("127.0.0.1", &sk_addr.sin_addr));
	CHECK(bind(sk_listen, (struct sockaddr *)&sk_addr, sizeof(sk_addr)));

	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLOUT | POLLHUP));
}
END_TEST()

FN_TEST(poll_listen)
{
	CHECK(listen(sk_listen, 3));

	TEST_RES(poll(&pfd, 1, 0), pfd.revents == 0);
}
END_TEST()

FN_TEST(poll_connect_close)
{
	sk_connect = CHECK(socket(PF_INET, SOCK_STREAM, 0));
	CHECK(connect(sk_connect, (struct sockaddr *)&sk_addr,
		      sizeof(sk_addr)));

	pfd.fd = sk_connect;
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLOUT));
	pfd.fd = sk_listen;
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLIN));

	close(sk_connect);
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLIN));

	struct sockaddr addr;
	socklen_t addrlen = sizeof(addr);
	int sk = CHECK(accept(sk_listen, &addr, &addrlen));
	pfd.fd = sk;
	TEST_RES(poll(&pfd, 1, 0),
		 pfd.revents == (POLLIN | POLLOUT | POLLRDHUP));
}
END_TEST()

FN_TEST(poll_connect_accept)
{
	sk_connect = CHECK(socket(PF_INET, SOCK_STREAM, 0));
	CHECK(connect(sk_connect, (struct sockaddr *)&sk_addr,
		      sizeof(sk_addr)));

	pfd.fd = sk_listen;
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLIN));

	struct sockaddr addr;
	socklen_t addrlen = sizeof(addr);
	sk_accept = CHECK(accept(sk_listen, &addr, &addrlen));

	pfd.fd = sk_accept;
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLOUT));
	pfd.fd = sk_connect;
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLOUT));
	pfd.fd = sk_listen;
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == 0);
}
END_TEST()

FN_TEST(poll_read_write)
{
	CHECK(write(sk_accept, buf, 1));

	pfd.fd = sk_connect;
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLIN | POLLOUT));

	TEST_RES(read(sk_connect, buf, 4096), _ret == 1);
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLOUT));

	CHECK(write(sk_connect, buf, 4096));
	CHECK(write(sk_connect, buf, 4096));

	// Ensure all data is transmitted.
	sleep(1);

	pfd.fd = sk_accept;
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLIN | POLLOUT));
	CHECK(read(sk_accept, buf, 4096));
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLIN | POLLOUT));
	CHECK(read(sk_accept, buf, 4096));
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLOUT));
}
END_TEST()

FN_TEST(poll_shutdown_read)
{
	CHECK(write(sk_connect, buf, 1));

	pfd.fd = sk_accept;
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLIN | POLLOUT));
	pfd.fd = sk_connect;
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLOUT));

	CHECK(shutdown(sk_accept, SHUT_RD));
	pfd.fd = sk_accept;
	TEST_RES(poll(&pfd, 1, 0),
		 pfd.revents == (POLLIN | POLLOUT | POLLRDHUP));
	pfd.fd = sk_connect;
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLOUT));

	CHECK(write(sk_connect, buf, 1));
	CHECK(read(sk_accept, buf, 4096));

	pfd.fd = sk_accept;
	TEST_RES(poll(&pfd, 1, 0),
		 pfd.revents == (POLLIN | POLLOUT | POLLRDHUP));

	CHECK(shutdown(sk_connect, SHUT_RD));
	TEST_RES(poll(&pfd, 1, 0),
		 pfd.revents == (POLLIN | POLLOUT | POLLRDHUP));
	pfd.fd = sk_connect;
	TEST_RES(poll(&pfd, 1, 0),
		 pfd.revents == (POLLIN | POLLOUT | POLLRDHUP));
}
END_TEST()

void renew_connect_and_accept()
{
	close(sk_connect);
	close(sk_accept);

	sk_connect = CHECK(socket(PF_INET, SOCK_STREAM, 0));
	CHECK(connect(sk_connect, (struct sockaddr *)&sk_addr,
		      sizeof(sk_addr)));

	struct sockaddr addr;
	socklen_t addrlen = sizeof(addr);
	sk_accept = CHECK(accept(sk_listen, &addr, &addrlen));
}

FN_TEST(poll_shutdown_write)
{
	renew_connect_and_accept();

	CHECK(write(sk_connect, buf, 1));

	pfd.fd = sk_accept;
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLIN | POLLOUT));
	pfd.fd = sk_connect;
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLOUT));

	CHECK(shutdown(sk_accept, SHUT_WR));
	pfd.fd = sk_accept;
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLIN | POLLOUT));
	pfd.fd = sk_connect;
	TEST_RES(poll(&pfd, 1, 0),
		 pfd.revents == (POLLIN | POLLOUT | POLLRDHUP));

	CHECK(write(sk_connect, buf, 1));
	CHECK(read(sk_accept, buf, 4096));

	TEST_RES(poll(&pfd, 1, 0),
		 pfd.revents == (POLLIN | POLLOUT | POLLRDHUP));

	CHECK(write(sk_connect, buf, 1));
	CHECK(read(sk_accept, buf, 4096));

	TEST_RES(poll(&pfd, 1, 0),
		 pfd.revents == (POLLIN | POLLOUT | POLLRDHUP));

	CHECK(shutdown(sk_connect, SHUT_WR));

	pfd.fd = sk_connect;
	TEST_RES(poll(&pfd, 1, 0),
		 pfd.revents == (POLLIN | POLLOUT | POLLRDHUP | POLLHUP));
	pfd.fd = sk_accept;
	TEST_RES(poll(&pfd, 1, 0),
		 pfd.revents == (POLLIN | POLLOUT | POLLRDHUP | POLLHUP));
}
END_TEST()

FN_TEST(poll_shutdown_readwrite)
{
	renew_connect_and_accept();

	CHECK(write(sk_connect, buf, 1));

	pfd.fd = sk_accept;
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLIN | POLLOUT));
	pfd.fd = sk_connect;
	TEST_RES(poll(&pfd, 1, 0), pfd.revents == (POLLOUT));

	CHECK(shutdown(sk_accept, SHUT_RDWR));
	pfd.fd = sk_accept;
	TEST_RES(poll(&pfd, 1, 0),
		 pfd.revents == (POLLIN | POLLOUT | POLLRDHUP | POLLHUP));
	pfd.fd = sk_connect;
	TEST_RES(poll(&pfd, 1, 0),
		 pfd.revents == (POLLIN | POLLOUT | POLLRDHUP));

	CHECK(read(sk_connect, buf, 4096));
	pfd.fd = sk_connect;
	TEST_RES(poll(&pfd, 1, 0),
		 pfd.revents == (POLLIN | POLLOUT | POLLRDHUP));

	CHECK(write(sk_connect, buf, 4096));

	// TODO: The following test cannot be passed on Asterinas due to the following reasons:
	// 1. On Linux, an RST packet is generated when attempting to write to a closed socket.
	//    However, Asterinas currently does not generate this packet.
	// 2. RST packets cause a POLLERR on Linux, but Asterinas currently lack support for this.

	// TEST_RES(poll(&pfd, 1, 0),
	// 	 pfd.revents ==
	// 		 (POLLIN | POLLOUT | POLLRDHUP | POLLHUP | POLLERR));
}
END_TEST()

FN_TEST(poll_close)
{
	renew_connect_and_accept();

	CHECK(write(sk_accept, buf, 1));

	close(sk_accept);
	pfd.fd = sk_connect;
	TEST_RES(poll(&pfd, 1, 0),
		 pfd.revents == (POLLIN | POLLOUT | POLLRDHUP));

	CHECK(shutdown(sk_connect, SHUT_RDWR));
	TEST_RES(poll(&pfd, 1, 0),
		 pfd.revents == (POLLIN | POLLOUT | POLLRDHUP | POLLHUP));
}
END_TEST()

FN_TEST(read_shutdown_read)
{
	renew_connect_and_accept();

	shutdown(sk_accept, SHUT_RD);
	pfd.fd = sk_accept;
	TEST_RES(poll(&pfd, 1, 0),
		 pfd.revents == (POLLIN | POLLRDHUP | POLLOUT));
	TEST_RES(read(sk_accept, buf, 4096), _ret == 0);

	TEST_RES(write(sk_connect, buf, 1), _ret == 1);
	TEST_RES(poll(&pfd, 1, 0),
		 pfd.revents == (POLLIN | POLLRDHUP | POLLOUT));
	TEST_RES(read(sk_accept, buf, 4096), _ret == 1);
	TEST_RES(poll(&pfd, 1, 0),
		 pfd.revents == (POLLIN | POLLRDHUP | POLLOUT));
	TEST_RES(read(sk_accept, buf, 4096), _ret == 0);
	TEST_RES(poll(&pfd, 1, 0),
		 pfd.revents == (POLLIN | POLLRDHUP | POLLOUT));
}
END_TEST()
