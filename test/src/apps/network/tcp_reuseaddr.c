// SPDX-License-Identifier: MPL-2.0

#include <unistd.h>
#include <sys/signal.h>
#include <sys/socket.h>
#include <sys/poll.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <fcntl.h>

#include "../test.h"

int sock1;
int sock2;
int sock3;

struct sockaddr_in addr;
socklen_t addrlen;

FN_SETUP(init)
{
	sock1 = CHECK(socket(AF_INET, SOCK_STREAM, 0));
	sock2 = CHECK(socket(AF_INET, SOCK_STREAM, 0));
	sock3 = CHECK(socket(AF_INET, SOCK_STREAM, 0));
	addr.sin_family = AF_INET;
	CHECK(inet_aton("127.0.0.1", &addr.sin_addr));
	addrlen = sizeof(addr);
}
END_SETUP()

FN_TEST(bind_to_ephemeral)
{
	int option = 1;
	TEST_SUCC(setsockopt(sock1, SOL_SOCKET, SO_REUSEADDR, &option,
			     sizeof(option)));
	addr.sin_port = 0;
	TEST_SUCC(bind(sock1, (struct sockaddr *)&addr, addrlen));

	TEST_SUCC(getsockname(sock1, (struct sockaddr *)&addr, &addrlen));

	TEST_ERRNO(bind(sock2, (struct sockaddr *)&addr, addrlen), EADDRINUSE);

	TEST_SUCC(setsockopt(sock2, SOL_SOCKET, SO_REUSEADDR, &option,
			     sizeof(option)));
	TEST_SUCC(bind(sock2, (struct sockaddr *)&addr, addrlen));
}
END_TEST()

void renew_socks()
{
	CHECK(close(sock1));
	CHECK(close(sock2));
	CHECK(close(sock3));
	sock1 = CHECK(socket(AF_INET, SOCK_STREAM, 0));
	sock2 = CHECK(socket(AF_INET, SOCK_STREAM, 0));
	sock3 = CHECK(socket(AF_INET, SOCK_STREAM, 0));
}

FN_TEST(bind_to_listening_port)
{
	renew_socks();

	int option = 1;
	TEST_SUCC(setsockopt(sock1, SOL_SOCKET, SO_REUSEADDR, &option,
			     sizeof(option)));
	addr.sin_port = 0;
	TEST_SUCC(bind(sock1, (struct sockaddr *)&addr, addrlen));

	TEST_SUCC(listen(sock1, 1));

	TEST_SUCC(getsockname(sock1, (struct sockaddr *)&addr, &addrlen));

	TEST_ERRNO(bind(sock2, (struct sockaddr *)&addr, addrlen), EADDRINUSE);

	TEST_SUCC(setsockopt(sock2, SOL_SOCKET, SO_REUSEADDR, &option,
			     sizeof(option)));

	// Currently, Asterinas does not check whether the port is already in use
	// by a listening socket when binding, so this test will fail.
	// TEST_ERRNO(bind(sock2, (struct sockaddr *)&addr, addrlen), EADDRINUSE);
}
END_TEST()

FN_TEST(listen_on_the_same_port)
{
	renew_socks();

	int option = 1;
	TEST_SUCC(setsockopt(sock1, SOL_SOCKET, SO_REUSEADDR, &option,
			     sizeof(option)));
	addr.sin_port = 0;
	TEST_SUCC(bind(sock1, (struct sockaddr *)&addr, addrlen));

	TEST_SUCC(getsockname(sock1, (struct sockaddr *)&addr, &addrlen));

	TEST_SUCC(setsockopt(sock2, SOL_SOCKET, SO_REUSEADDR, &option,
			     sizeof(option)));
	TEST_SUCC(bind(sock2, (struct sockaddr *)&addr, addrlen));

	TEST_SUCC(listen(sock1, 1));
	TEST_ERRNO(listen(sock2, 1), EADDRINUSE);

	TEST_SUCC(setsockopt(sock3, SOL_SOCKET, SO_REUSEADDR, &option,
			     sizeof(option)));

	// Currently, Asterinas does not check whether the port is already in use
	// by a listening socket when binding, so this test will fail.
	// TEST_ERRNO(bind(sock3, (struct sockaddr *)&addr, addrlen), EADDRINUSE);
}
END_TEST()

FN_TEST(bind_to_connected_port)
{
	renew_socks();

	int option = 1;
	TEST_SUCC(setsockopt(sock1, SOL_SOCKET, SO_REUSEADDR, &option,
			     sizeof(option)));
	addr.sin_port = 0;
	TEST_SUCC(bind(sock1, (struct sockaddr *)&addr, addrlen));

	TEST_SUCC(listen(sock1, 3));

	TEST_SUCC(getsockname(sock1, (struct sockaddr *)&addr, &addrlen));

	TEST_SUCC(setsockopt(sock2, SOL_SOCKET, SO_REUSEADDR, &option,
			     sizeof(option)));
	TEST_SUCC(connect(sock2, (struct sockaddr *)&addr, addrlen));

	struct sockaddr_in sock2_addr;
	TEST_SUCC(getsockname(sock2, (struct sockaddr *)&sock2_addr, &addrlen));

	int sock3 = TEST_SUCC(socket(AF_INET, SOCK_STREAM, 0));

	TEST_ERRNO(bind(sock3, (struct sockaddr *)&sock2_addr, addrlen),
		   EADDRINUSE);

	TEST_SUCC(setsockopt(sock3, SOL_SOCKET, SO_REUSEADDR, &option,
			     sizeof(option)));
	TEST_SUCC(bind(sock3, (struct sockaddr *)&sock2_addr, addrlen));

	TEST_ERRNO(connect(sock2, (struct sockaddr *)&addr, addrlen), EISCONN);

	TEST_SUCC(close(sock3));
}
END_TEST()

FN_TEST(enable_reuse_after_bound)
{
	renew_socks();

	int option = 0;
	TEST_SUCC(setsockopt(sock1, SOL_SOCKET, SO_REUSEADDR, &option,
			     sizeof(option)));
	addr.sin_port = 0;
	TEST_SUCC(bind(sock1, (struct sockaddr *)&addr, addrlen));

	TEST_SUCC(getsockname(sock1, (struct sockaddr *)&addr, &addrlen));

	option = 1;
	TEST_SUCC(setsockopt(sock2, SOL_SOCKET, SO_REUSEADDR, &option,
			     sizeof(option)));
	TEST_ERRNO(bind(sock2, (struct sockaddr *)&addr, addrlen), EADDRINUSE);

	TEST_SUCC(setsockopt(sock1, SOL_SOCKET, SO_REUSEADDR, &option,
			     sizeof(option)));
	TEST_SUCC(bind(sock2, (struct sockaddr *)&addr, addrlen));
}
END_TEST()

FN_TEST(disable_reuse_after_bound)
{
	renew_socks();

	int option = 1;
	TEST_SUCC(setsockopt(sock1, SOL_SOCKET, SO_REUSEADDR, &option,
			     sizeof(option)));
	addr.sin_port = 0;
	TEST_SUCC(bind(sock1, (struct sockaddr *)&addr, addrlen));

	option = 0;
	socklen_t option_len = sizeof(option);
	TEST_SUCC(setsockopt(sock1, SOL_SOCKET, SO_REUSEADDR, &option,
			     option_len));
	TEST_RES(getsockopt(sock1, SOL_SOCKET, SO_REUSEADDR, &option,
			    &option_len),
		 option == 0 && option_len == 4);

	TEST_SUCC(getsockname(sock1, (struct sockaddr *)&addr, &addrlen));

	// The following test succeeds on Linux because Linux does not allow disabling
	// SO_REUSEADDR for TCP sockets once the socket is bound. In contrast, Asterinas
	// enforces a stricter rule: the port can be reused only if all sockets bound to it
	// have port reuse enabled.
	// See the discussion at <https://github.com/asterinas/asterinas/pull/2277#discussion_r2230139244>.
	//
	// option = 1;
	// TEST_SUCC(setsockopt(sock2, SOL_SOCKET, SO_REUSEADDR, &option,
	// 		     sizeof(option)));
	// TEST_SUCC(bind(sock2, (struct sockaddr *)&addr, addrlen));
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(sock1));
	CHECK(close(sock2));
	CHECK(close(sock3));
}
END_SETUP()