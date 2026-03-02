// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <unistd.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include "../common/test.h"

#define SENDER_BIND_ADDR "127.0.0.1"
#define SENDER_PORT 12345

#define BROADCAST_ADDR "127.255.255.255"
#define RECEIVE_PORT 12346

#define MESSAGE "Hello from broadcast"
#define MESSAGE_LEN sizeof(MESSAGE)

int sender;
int receiver;
char buf[128] = { 0 };

struct sockaddr_in broadcast_addr;
struct sockaddr_in received_addr;
socklen_t addr_len = sizeof(struct sockaddr_in);

int broadcast_opt;
socklen_t broadcast_opt_len = sizeof(broadcast_opt);

FN_SETUP(create_and_bind)
{
	// Initialize the broadcast address
	CHECK(inet_aton(BROADCAST_ADDR, &broadcast_addr.sin_addr));
	broadcast_addr.sin_port = htons(RECEIVE_PORT);
	broadcast_addr.sin_family = AF_INET;

	// Bind sender to SENDER_BIND_ADDR:SENDER_PORT
	struct sockaddr_in sk_addr;
	sk_addr.sin_family = AF_INET;
	CHECK(inet_aton(SENDER_BIND_ADDR, &sk_addr.sin_addr));
	sk_addr.sin_port = htons(SENDER_PORT);
	sender = CHECK(socket(AF_INET, SOCK_DGRAM, 0));
	CHECK(bind(sender, (struct sockaddr *)&sk_addr, addr_len));

	// Bind receiver to BROADCAST_ADDR:RECEIVE_PORT
	receiver = CHECK(socket(AF_INET, SOCK_DGRAM, 0));
	// FIXME: Asterinas cannot support binding to broadcast addresses now.
	// So all below code related to receiver is commented out.
#ifndef __asterinas__
	CHECK(bind(receiver, (struct sockaddr *)&broadcast_addr, addr_len));
#endif
}
END_SETUP()

FN_TEST(enable_broadcast)
{
	TEST_RES(getsockopt(sender, SOL_SOCKET, SO_BROADCAST, &broadcast_opt,
			    &broadcast_opt_len),
		 broadcast_opt == 0 && broadcast_opt_len == 4);
	TEST_RES(getsockopt(receiver, SOL_SOCKET, SO_BROADCAST, &broadcast_opt,
			    &broadcast_opt_len),
		 broadcast_opt == 0 && broadcast_opt_len == 4);

	broadcast_opt = 100;
	TEST_SUCC(setsockopt(sender, SOL_SOCKET, SO_BROADCAST, &broadcast_opt,
			     broadcast_opt_len));
	broadcast_opt = -1;
	TEST_SUCC(setsockopt(receiver, SOL_SOCKET, SO_BROADCAST, &broadcast_opt,
			     broadcast_opt_len));

	TEST_RES(getsockopt(sender, SOL_SOCKET, SO_BROADCAST, &broadcast_opt,
			    &broadcast_opt_len),
		 broadcast_opt == 1 && broadcast_opt_len == 4);
	TEST_RES(getsockopt(receiver, SOL_SOCKET, SO_BROADCAST, &broadcast_opt,
			    &broadcast_opt_len),
		 broadcast_opt == 1 && broadcast_opt_len == 4);
}
END_TEST()

// Both sender and receiver have SO_BROADCAST enabled.
FN_TEST(basic_broadcast)
{
	TEST_SUCC(sendto(sender, MESSAGE, MESSAGE_LEN, 0,
			 (struct sockaddr *)&broadcast_addr, addr_len));

#ifndef __asterinas__
	TEST_SUCC(recvfrom(receiver, buf, sizeof(buf), 0,
			   (struct sockaddr *)&received_addr, &addr_len));
#endif
}
END_TEST()

// Sender attempts to send broadcast without SO_BROADCAST enabled.
FN_TEST(disable_sender_broadcast)
{
	broadcast_opt = 0;
	TEST_SUCC(setsockopt(sender, SOL_SOCKET, SO_BROADCAST, &broadcast_opt,
			     broadcast_opt_len));

	TEST_ERRNO(sendto(sender, MESSAGE, MESSAGE_LEN, 0,
			  (struct sockaddr *)&broadcast_addr, addr_len),
		   EACCES);

	TEST_ERRNO(connect(sender, (struct sockaddr *)&broadcast_addr,
			   addr_len),
		   EACCES);
	TEST_ERRNO(send(sender, MESSAGE, MESSAGE_LEN, 0), EDESTADDRREQ);
}
END_TEST()

// Sender enables SO_BROADCAST, receiver disables SO_BROADCAST.
FN_TEST(disable_receiver_broadcast)
{
	broadcast_opt = 1;
	TEST_SUCC(setsockopt(sender, SOL_SOCKET, SO_BROADCAST, &broadcast_opt,
			     broadcast_opt_len));

	broadcast_opt = 0;
	TEST_SUCC(setsockopt(receiver, SOL_SOCKET, SO_BROADCAST, &broadcast_opt,
			     broadcast_opt_len));

	TEST_SUCC(sendto(sender, MESSAGE, MESSAGE_LEN, 0,
			 (struct sockaddr *)&broadcast_addr, addr_len));

#ifndef __asterinas__
	TEST_SUCC(recvfrom(receiver, buf, sizeof(buf), 0,
			   (struct sockaddr *)&received_addr, &addr_len));
#endif
}
END_TEST()

// Connect sender to broadcast address, then send.
// Then disable SO_BROADCAST on sender and try to send again.
FN_TEST(connect_then_disable_broadcast)
{
	TEST_SUCC(
		connect(sender, (struct sockaddr *)&broadcast_addr, addr_len));
	TEST_SUCC(send(sender, MESSAGE, MESSAGE_LEN, 0));
#ifndef __asterinas__
	TEST_SUCC(recvfrom(receiver, buf, sizeof(buf), 0,
			   (struct sockaddr *)&received_addr, &addr_len));
#endif

	broadcast_opt = 0;
	TEST_SUCC(setsockopt(sender, SOL_SOCKET, SO_BROADCAST, &broadcast_opt,
			     broadcast_opt_len));

	TEST_SUCC(send(sender, MESSAGE, MESSAGE_LEN, 0));
#ifndef __asterinas__
	TEST_SUCC(recvfrom(receiver, buf, sizeof(buf), 0,
			   (struct sockaddr *)&received_addr, &addr_len));
#endif

	TEST_ERRNO(sendto(sender, MESSAGE, MESSAGE_LEN, 0,
			  (struct sockaddr *)&broadcast_addr, addr_len),
		   EACCES);

	TEST_ERRNO(connect(sender, (struct sockaddr *)&broadcast_addr,
			   addr_len),
		   EACCES);
	// FIXME: Asterinas cannot pass the following case.
	// The problem may be that we should invalidate the connected state
	// once the above connect fails.
#ifndef __asterinas__
	TEST_ERRNO(send(sender, MESSAGE, MESSAGE_LEN, 0), EACCES);
#endif
}
END_TEST()

FN_SETUP(cleanup)
{
	CHECK(close(sender));
	CHECK(close(receiver));
}
END_SETUP()
