// SPDX-License-Identifier: MPL-2.0

#include <unistd.h>
#include <netinet/ip.h>
#include <arpa/inet.h>

#include "../common/test.h"

static struct sockaddr_in sk_addr;

FN_SETUP(init)
{
	// This does not always work, but it is the most simple way
	// to drop the `CAP_NET_BIND_SERVICE` capability in common cases.
	if (getuid() == 0)
		CHECK(setuid(65534));

	sk_addr.sin_family = AF_INET;
	CHECK(inet_aton("127.0.0.1", &sk_addr.sin_addr));
}
END_SETUP()

FN_TEST(tcp_privileged_ports)
{
	int sk;

	sk = TEST_SUCC(socket(PF_INET, SOCK_STREAM, 0));

	sk_addr.sin_port = htons(1023);
	TEST_ERRNO(bind(sk, (struct sockaddr *)&sk_addr, sizeof(sk_addr)),
		   EACCES);

	sk_addr.sin_port = htons(1024);
	TEST_SUCC(bind(sk, (struct sockaddr *)&sk_addr, sizeof(sk_addr)));

	TEST_SUCC(close(sk));
}
END_TEST()

FN_TEST(udp_privileged_ports)
{
	int sk;

	sk = TEST_SUCC(socket(PF_INET, SOCK_DGRAM, 0));

	sk_addr.sin_port = htons(1023);
	TEST_ERRNO(bind(sk, (struct sockaddr *)&sk_addr, sizeof(sk_addr)),
		   EACCES);

	sk_addr.sin_port = htons(1024);
	TEST_SUCC(bind(sk, (struct sockaddr *)&sk_addr, sizeof(sk_addr)));

	TEST_SUCC(close(sk));
}
END_TEST()
