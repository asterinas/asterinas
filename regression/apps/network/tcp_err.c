// SPDX-License-Identifier: MPL-2.0

#include <unistd.h>
#include <sys/signal.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>

#include "test.h"

static struct sockaddr_in sk_addr;

#define C_PORT htons(0x1234)
#define S_PORT htons(0x1235)

FN_SETUP(general)
{
	sk_addr.sin_family = AF_INET;
	sk_addr.sin_port = htons(8080);
	CHECK(inet_aton("127.0.0.1", &sk_addr.sin_addr));

	signal(SIGPIPE, SIG_IGN);
}
END_SETUP()

static int sk_unbound;
static int sk_bound;
static int sk_listen;
static int sk_connected;
static int sk_accepted;

FN_SETUP(unbound)
{
	sk_unbound = CHECK(socket(PF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0));
}
END_SETUP()

FN_SETUP(bound)
{
	sk_bound = CHECK(socket(PF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0));

	sk_addr.sin_port = C_PORT;
	CHECK(bind(sk_bound, (struct sockaddr *)&sk_addr, sizeof(sk_addr)));
}
END_SETUP()

FN_SETUP(listen)
{
	sk_listen = CHECK(socket(PF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0));

	sk_addr.sin_port = S_PORT;
	CHECK(bind(sk_listen, (struct sockaddr *)&sk_addr, sizeof(sk_addr)));

	CHECK(listen(sk_listen, 2));
}
END_SETUP()

FN_SETUP(connected)
{
	sk_connected = CHECK(socket(PF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0));

	sk_addr.sin_port = S_PORT;
	CHECK_WITH(connect(sk_connected, (struct sockaddr *)&sk_addr,
			   sizeof(sk_addr)),
		   _ret == 0 || errno == EINPROGRESS);
}
END_SETUP()

FN_SETUP(accpected)
{
	struct sockaddr addr;
	socklen_t addrlen = sizeof(addr);

	do {
		sk_accepted = CHECK_WITH(accept(sk_listen, &addr, &addrlen),
					 _ret >= 0 || errno == EAGAIN);
	} while (sk_accepted < 0);
}
END_SETUP()
