// SPDX-License-Identifier: MPL-2.0

#include <netinet/in.h>
#include <poll.h>
#include <sys/socket.h>
#include <unistd.h>

#include "../common/test.h"

#define POLL_TIMEOUT_MS 1000
#define TCP_SETTLE_USEC 100000

FN_TEST(tcp_accept_after_reset)
{
	int listener = TEST_SUCC(socket(AF_INET, SOCK_STREAM, 0));
	struct sockaddr_in listen_addr = {
		.sin_family = AF_INET,
		.sin_addr.s_addr = htonl(INADDR_LOOPBACK),
	};
	socklen_t addrlen = sizeof(listen_addr);

	TEST_SUCC(bind(listener, (struct sockaddr *)&listen_addr,
		       sizeof(listen_addr)));
	TEST_SUCC(listen(listener, 2));
	TEST_RES(getsockname(listener, (struct sockaddr *)&listen_addr,
			     &addrlen),
		 addrlen == sizeof(listen_addr));

	int client = TEST_SUCC(socket(AF_INET, SOCK_STREAM, 0));
	struct sockaddr_in client_addr;

	TEST_SUCC(connect(client, (struct sockaddr *)&listen_addr,
			  sizeof(listen_addr)));
	addrlen = sizeof(client_addr);
	TEST_RES(getsockname(client, (struct sockaddr *)&client_addr, &addrlen),
		 addrlen == sizeof(client_addr));

	// Wait until the handshake completes before resetting the connection.
	struct pollfd pfd = { .fd = listener, .events = POLLIN };
	TEST_RES(poll(&pfd, 1, POLL_TIMEOUT_MS),
		 _ret == 1 && pfd.revents == POLLIN);

	// Allow the RST to reach the listener before accepting the connection.
	struct linger linger = { .l_onoff = 1, .l_linger = 0 };
	TEST_SUCC(setsockopt(client, SOL_SOCKET, SO_LINGER, &linger,
			     sizeof(linger)));
	TEST_SUCC(close(client));
	TEST_SUCC(usleep(TCP_SETTLE_USEC));

	struct sockaddr_in accepted_addr = { 0 };
	addrlen = sizeof(accepted_addr);
	int accepted = TEST_RES(
		accept(listener, (struct sockaddr *)&accepted_addr, &addrlen),
		addrlen == sizeof(accepted_addr) &&
			accepted_addr.sin_family == AF_INET &&
			accepted_addr.sin_addr.s_addr ==
				client_addr.sin_addr.s_addr &&
			accepted_addr.sin_port == client_addr.sin_port);

	// getpeername() rejects a closed connection. It must not
	// consume the pending reset error, which is still reported by recv().
	struct sockaddr_in peer_addr;
	char byte;
	addrlen = sizeof(peer_addr);
	TEST_ERRNO(getpeername(accepted, (struct sockaddr *)&peer_addr,
			       &addrlen),
		   ENOTCONN);
	TEST_ERRNO(recv(accepted, &byte, sizeof(byte), 0), ECONNRESET);
	TEST_RES(recv(accepted, &byte, sizeof(byte), 0), _ret == 0);
	TEST_ERRNO(getpeername(accepted, (struct sockaddr *)&peer_addr,
			       &addrlen),
		   ENOTCONN);

	TEST_SUCC(close(accepted));
	TEST_SUCC(close(listener));
}
END_TEST()
