// SPDX-License-Identifier: MPL-2.0

#include <unistd.h>
#include <sys/signal.h>
#include <sys/socket.h>
#include <sys/poll.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <netinet/ip.h>
#include <netinet/udp.h>
#include <fcntl.h>
#include <stdbool.h>

#include "test.h"

#define C_PORT htons(0x1234)

static struct sockaddr_in dest_addr;
static struct sockaddr_in recv_addr;

static int sk_unbound;
static int sk_bound;
static int sk_connected;
static int sk_no_iphdrincl;
static int sk_with_iphdrincl;

unsigned short checksum(void *b, int len)
{
	unsigned short *buf = b;
	unsigned int sum = 0;
	unsigned short result;

	for (sum = 0; len > 1; len -= 2) {
		sum += *buf++;
	}

	if (len == 1) {
		sum += *(unsigned char *)buf;
	}

	sum = (sum >> 16) + (sum & 0xFFFF);
	sum += (sum >> 16);
	result = ~sum;
	return result;
}

FN_SETUP(general)
{
	memset(&dest_addr, 0, sizeof(dest_addr));
	dest_addr.sin_family = AF_INET;
	dest_addr.sin_port = C_PORT;
	CHECK(inet_aton("127.0.0.1", &dest_addr.sin_addr));
	memset(&recv_addr, 0, sizeof(recv_addr));
	signal(SIGPIPE, SIG_IGN);
}
END_SETUP()

FN_SETUP(unbound)
{
	sk_unbound =
		CHECK(socket(AF_INET, SOCK_RAW | SOCK_NONBLOCK, IPPROTO_UDP));
}
END_SETUP()

FN_SETUP(bound)
{
	sk_bound =
		CHECK(socket(AF_INET, SOCK_RAW | SOCK_NONBLOCK, IPPROTO_UDP));

	dest_addr.sin_port = C_PORT;
	CHECK(bind(sk_bound, (struct sockaddr *)&dest_addr, sizeof(dest_addr)));
}
END_SETUP()

FN_SETUP(connected)
{
	sk_connected =
		CHECK(socket(AF_INET, SOCK_RAW | SOCK_NONBLOCK, IPPROTO_UDP));

	dest_addr.sin_port = C_PORT;
	CHECK(connect(sk_connected, (struct sockaddr *)&dest_addr,
		      sizeof(dest_addr)));
}
END_SETUP()

FN_SETUP(ip_hdr_incl)
{
	sk_no_iphdrincl =
		CHECK(socket(PF_INET, SOCK_RAW | SOCK_NONBLOCK, IPPROTO_UDP));
	sk_with_iphdrincl =
		CHECK(socket(PF_INET, SOCK_RAW | SOCK_NONBLOCK, IPPROTO_UDP));
	int optval = 1;
	CHECK(setsockopt(sk_with_iphdrincl, IPPROTO_IP, IP_HDRINCL, &optval,
			 sizeof(optval)) == 0);
}
END_SETUP()

FN_TEST(getsockname)
{
	struct sockaddr_in saddr = { .sin_port = 0xbeef };
	struct sockaddr *psaddr = (struct sockaddr *)&saddr;
	socklen_t addrlen = 0;

	TEST_RES(getsockname(sk_unbound, psaddr, &addrlen),
		 addrlen == sizeof(saddr) && saddr.sin_port == 0xbeef);

	TEST_RES(getsockname(sk_unbound, psaddr, &addrlen),
		 addrlen == sizeof(saddr) &&
			 saddr.sin_addr.s_addr == inet_addr("0.0.0.0"));

	TEST_RES(getsockname(sk_bound, psaddr, &addrlen),
		 addrlen == sizeof(saddr) &&
			 saddr.sin_addr.s_addr == inet_addr("127.0.0.1"));

	TEST_RES(getsockname(sk_connected, psaddr, &addrlen),
		 addrlen == sizeof(saddr) &&
			 saddr.sin_addr.s_addr == inet_addr("127.0.0.1") &&
			 saddr.sin_port != C_PORT); //saddr.sin_port != C_PORT);
}
END_TEST()

FN_TEST(getpeername)
{
	struct sockaddr_in saddr = { .sin_port = 0xbeef };
	struct sockaddr *psaddr = (struct sockaddr *)&saddr;
	socklen_t addrlen = sizeof(saddr);

	TEST_ERRNO(getpeername(sk_unbound, psaddr, &addrlen), ENOTCONN);

	TEST_ERRNO(getpeername(sk_bound, psaddr, &addrlen), ENOTCONN);

	TEST_RES(getpeername(sk_connected, psaddr, &addrlen),
		 addrlen == sizeof(saddr) &&
			 saddr.sin_addr.s_addr == inet_addr("127.0.0.1") &&
			 saddr.sin_port == C_PORT);
}
END_TEST()

FN_TEST(send)
{
	char send_buf[4096];
	struct udphdr udph;

	memset(send_buf, 0, sizeof(send_buf));

	udph.source = htons(12345);
	udph.dest = C_PORT;
	udph.len = htons(sizeof(struct udphdr) + 1);
	udph.check = 0;

	memcpy(send_buf, &udph, sizeof(struct udphdr));
	send_buf[sizeof(struct udphdr)] = 'a';

	TEST_ERRNO(send(sk_unbound, send_buf, sizeof(struct udphdr) + 1, 0),
		   EDESTADDRREQ);

	TEST_ERRNO(send(sk_bound, send_buf, sizeof(struct udphdr) + 1, 0),
		   EDESTADDRREQ);
}
END_TEST()

FN_TEST(recv)
{
	char recv_buf[4096];

	TEST_ERRNO(recv(sk_unbound, recv_buf, sizeof(struct udphdr) + 1, 0),
		   EAGAIN);

	TEST_ERRNO(recv(sk_bound, recv_buf, sizeof(struct udphdr) + 1, 0),
		   EAGAIN);

	TEST_ERRNO(recv(sk_connected, recv_buf, sizeof(struct udphdr) + 1, 0),
		   EAGAIN);
}
END_TEST()

FN_TEST(bind)
{
	struct sockaddr *psaddr = (struct sockaddr *)&dest_addr;
	socklen_t addrlen = sizeof(dest_addr);

	TEST_ERRNO(bind(sk_unbound, psaddr, addrlen - 1), EINVAL);

	// FIXME: The test will fail in Asterinas since it doesn't not support multiple calls to bind() with one socket.
	// TEST_SUCC(bind(sk_bound, psaddr, addrlen));

	TEST_ERRNO(bind(sk_connected, psaddr, addrlen), EINVAL);
}
END_TEST()

FN_TEST(listen)
{
	TEST_ERRNO(listen(sk_unbound, 2), EOPNOTSUPP);

	TEST_ERRNO(listen(sk_bound, 2), EOPNOTSUPP);

	TEST_ERRNO(listen(sk_connected, 2), EOPNOTSUPP);
}
END_TEST()

FN_TEST(accept)
{
	struct sockaddr_in saddr;
	struct sockaddr *psaddr = (struct sockaddr *)&saddr;
	socklen_t addrlen = sizeof(saddr);

	TEST_ERRNO(accept(sk_unbound, psaddr, &addrlen), EOPNOTSUPP);

	TEST_ERRNO(accept(sk_bound, psaddr, &addrlen), EOPNOTSUPP);

	TEST_ERRNO(accept(sk_connected, psaddr, &addrlen), EOPNOTSUPP);
}
END_TEST()

FN_TEST(poll)
{
	struct pollfd pfd = { .events = POLLIN | POLLOUT };

	pfd.fd = sk_unbound;
	TEST_RES(poll(&pfd, 1, 0),
		 (pfd.revents & (POLLIN | POLLOUT)) == POLLOUT);

	pfd.fd = sk_bound;
	TEST_RES(poll(&pfd, 1, 0),
		 (pfd.revents & (POLLIN | POLLOUT)) == POLLOUT);

	pfd.fd = sk_connected;
	TEST_RES(poll(&pfd, 1, 0),
		 (pfd.revents & (POLLIN | POLLOUT)) == POLLOUT);
}
END_TEST()

FN_TEST(connect)
{
	struct sockaddr *psaddr = (struct sockaddr *)&dest_addr;
	socklen_t addrlen = sizeof(dest_addr);

	TEST_SUCC(connect(sk_connected, psaddr, addrlen));
}
END_TEST()

void set_blocking(int sockfd)
{
	int flags = CHECK(fcntl(sockfd, F_GETFL, 0));
	CHECK(fcntl(sockfd, F_SETFL, flags & (~O_NONBLOCK)));
}

FN_SETUP(enter_blocking_mode)
{
	set_blocking(sk_connected);
	set_blocking(sk_bound);
}
END_SETUP()

FN_TEST(sendto_and_recvfrom)
{
	char send_buf[4096];
	char recv_buf[4096];
	socklen_t addr_len = sizeof(recv_addr);
	struct udphdr udph;

	memset(send_buf, 0, sizeof(send_buf));
	memset(recv_buf, 0, sizeof(recv_buf));

	udph.source = htons(12345);
	udph.dest = C_PORT;
	udph.len = htons(sizeof(struct udphdr) + 1);
	udph.check = 0;

	memcpy(send_buf, &udph, sizeof(struct udphdr));
	send_buf[sizeof(struct udphdr)] = 'a';
	struct udphdr *recv_udph =
		(struct udphdr *)(recv_buf + sizeof(struct iphdr));

	// TEST CASE 1: Send one message and receive one message
	TEST_RES(sendto(sk_no_iphdrincl, send_buf, htons(udph.len), 0,
			(struct sockaddr *)&dest_addr, sizeof(dest_addr)),
		 _ret == sizeof(struct udphdr) + 1);

	TEST_RES(
		recvfrom(sk_no_iphdrincl, recv_buf, sizeof(recv_buf), 0,
			 (struct sockaddr *)&recv_addr, &addr_len),
		_ret == sizeof(struct iphdr) + sizeof(struct udphdr) + 1 &&
			recv_buf[sizeof(struct iphdr) + sizeof(struct udphdr)] ==
				'a' &&
			ntohs(recv_udph->dest) == ntohs(udph.dest));

	// TEST CASE 2: Send two messages and receive two messages
	memcpy(send_buf, &udph, sizeof(struct udphdr));
	send_buf[sizeof(struct udphdr)] = 'b';

	TEST_RES(sendto(sk_no_iphdrincl, send_buf, htons(udph.len), 0,
			(struct sockaddr *)&dest_addr, sizeof(dest_addr)),
		 _ret == sizeof(struct udphdr) + 1);

	memcpy(send_buf, &udph, sizeof(struct udphdr));
	send_buf[sizeof(struct udphdr)] = 'c';

	TEST_RES(sendto(sk_no_iphdrincl, send_buf, htons(udph.len), 0,
			(struct sockaddr *)&dest_addr, sizeof(dest_addr)),
		 _ret == sizeof(struct udphdr) + 1);

	memset(recv_buf, 0, sizeof(recv_buf));
	TEST_RES(
		recvfrom(sk_no_iphdrincl, recv_buf, sizeof(recv_buf), 0,
			 (struct sockaddr *)&recv_addr, &addr_len),
		_ret == sizeof(struct iphdr) + sizeof(struct udphdr) + 1 &&
			recv_buf[sizeof(struct iphdr) + sizeof(struct udphdr)] ==
				'b' &&
			ntohs(recv_udph->dest) == ntohs(udph.dest));

	memset(recv_buf, 0, sizeof(recv_buf));
	TEST_RES(
		recvfrom(sk_no_iphdrincl, recv_buf, sizeof(recv_buf), 0,
			 (struct sockaddr *)&recv_addr, &addr_len),
		_ret == sizeof(struct iphdr) + sizeof(struct udphdr) + 1 &&
			recv_buf[sizeof(struct iphdr) + sizeof(struct udphdr)] ==
				'c' &&
			ntohs(recv_udph->dest) == ntohs(udph.dest));
}
END_TEST()

FN_TEST(sendmsg_and_recvmsg)
{
	char send_buf[4096];
	char recv_buf[4096];
	struct udphdr udph;

	memset(send_buf, 0, sizeof(send_buf));
	memset(recv_buf, 0, sizeof(recv_buf));

	udph.source = htons(12345);
	udph.dest = C_PORT;
	udph.len = htons(sizeof(struct udphdr) + 1);
	udph.check = 0;

	memcpy(send_buf, &udph, sizeof(struct udphdr));
	send_buf[sizeof(struct udphdr)] = 'a';
	struct udphdr *recv_udph =
		(struct udphdr *)(recv_buf + sizeof(struct iphdr));

	struct msghdr msg_send = { 0 };
	struct msghdr msg_recv = { 0 };
	struct iovec iov_send = { 0 };
	struct iovec iov_recv = { 0 };

	// TEST CASE 1: Send one message and receive one message
	iov_send.iov_base = send_buf;
	iov_send.iov_len = htons(udph.len);
	msg_send.msg_name = &dest_addr;
	msg_send.msg_namelen = sizeof(dest_addr);
	msg_send.msg_iov = &iov_send;
	msg_send.msg_iovlen = 1;

	TEST_RES(sendmsg(sk_no_iphdrincl, &msg_send, 0),
		 _ret == sizeof(struct udphdr) + 1);

	char name_buf[sizeof(struct sockaddr_in)];
	msg_recv.msg_name = name_buf;
	msg_recv.msg_namelen = sizeof(name_buf);
	iov_recv.iov_base = recv_buf;
	iov_recv.iov_len = sizeof(recv_buf);
	msg_recv.msg_iov = &iov_recv;
	msg_recv.msg_iovlen = 1;

	TEST_RES(
		recvmsg(sk_no_iphdrincl, &msg_recv, 0),
		_ret == sizeof(struct iphdr) + sizeof(struct udphdr) + 1 &&
			recv_buf[sizeof(struct iphdr) + sizeof(struct udphdr)] ==
				'a' &&
			ntohs(recv_udph->dest) == ntohs(udph.dest));

	// Verify msg.msg_name (address and port)
	struct sockaddr_in *recv_name = (struct sockaddr_in *)msg_recv.msg_name;
	TEST_RES(recv_name->sin_addr.s_addr == dest_addr.sin_addr.s_addr, true);
	TEST_RES(recv_name->sin_port == dest_addr.sin_port, true);

	// TEST CASE 2: Send two messages and receive two messages
	memcpy(send_buf, &udph, sizeof(struct udphdr));
	send_buf[sizeof(struct udphdr)] = 'b';

	TEST_RES(sendmsg(sk_no_iphdrincl, &msg_send, 0),
		 _ret == sizeof(struct udphdr) + 1);

	memcpy(send_buf, &udph, sizeof(struct udphdr));
	send_buf[sizeof(struct udphdr)] = 'c';

	TEST_RES(sendmsg(sk_no_iphdrincl, &msg_send, 0),
		 _ret == sizeof(struct udphdr) + 1);

	memset(recv_buf, 0, sizeof(recv_buf));
	TEST_RES(
		recvmsg(sk_no_iphdrincl, &msg_recv, 0),
		_ret == sizeof(struct iphdr) + sizeof(struct udphdr) + 1 &&
			recv_buf[sizeof(struct iphdr) + sizeof(struct udphdr)] ==
				'b' &&
			ntohs(recv_udph->dest) == ntohs(udph.dest));

	// Verify msg.msg_name (address and port) for second message
	recv_name = (struct sockaddr_in *)msg_recv.msg_name;
	TEST_RES(recv_name->sin_addr.s_addr == dest_addr.sin_addr.s_addr, true);
	TEST_RES(recv_name->sin_port == dest_addr.sin_port, true);

	memset(recv_buf, 0, sizeof(recv_buf));
	TEST_RES(
		recvmsg(sk_no_iphdrincl, &msg_recv, 0),
		_ret == sizeof(struct iphdr) + sizeof(struct udphdr) + 1 &&
			recv_buf[sizeof(struct iphdr) + sizeof(struct udphdr)] ==
				'c' &&
			ntohs(recv_udph->dest) == ntohs(udph.dest));

	// Verify msg.msg_name (address and port) for third message
	recv_name = (struct sockaddr_in *)msg_recv.msg_name;
	TEST_RES(recv_name->sin_addr.s_addr == dest_addr.sin_addr.s_addr, true);
	TEST_RES(recv_name->sin_port == dest_addr.sin_port, true);
}
END_TEST()

FN_TEST(sendto_and_recvfrom_with_iphdrincl)
{
	char send_buf[4096];
	char recv_buf[4096];
	socklen_t addr_len = sizeof(recv_addr);

	struct iphdr iph;
	struct udphdr udph;

	memset(send_buf, 0, sizeof(send_buf));
	memset(recv_buf, 0, sizeof(recv_buf));
	iph.ihl = 5;
	iph.version = 4;
	iph.tos = 0;
	iph.tot_len = htons(sizeof(struct iphdr) + sizeof(struct udphdr) + 1);
	iph.id = htonl(54321);
	iph.frag_off = 0;
	iph.ttl = 255;
	iph.protocol = IPPROTO_UDP;
	iph.check = 0;
	iph.saddr = INADDR_ANY;
	iph.daddr = dest_addr.sin_addr.s_addr;
	iph.check = checksum((unsigned short *)&iph, sizeof(struct iphdr));

	udph.source = htons(54321);
	udph.dest = C_PORT;
	udph.len = htons(sizeof(struct udphdr) + 1);
	udph.check = 0;
	struct udphdr *recv_udph =
		(struct udphdr *)(recv_buf + sizeof(struct iphdr));

	// Avoid interference from previous packets on the network.
	for (int i = 0; i < 9; i++) {
		if (recvfrom(sk_with_iphdrincl, recv_buf, sizeof(recv_buf), 0,
			     (struct sockaddr *)&recv_addr, &addr_len) <= 0)
			break;
	}
	sleep(1);

	// TEST CASE 1: Send one message and receive one message
	memcpy(send_buf, &iph, sizeof(struct iphdr));
	memcpy(send_buf + sizeof(struct iphdr), &udph, sizeof(struct udphdr));
	send_buf[sizeof(struct iphdr) + sizeof(struct udphdr)] = 'd';
	TEST_RES(sendto(sk_with_iphdrincl, send_buf, htons(iph.tot_len), 0,
			(struct sockaddr *)&dest_addr, sizeof(dest_addr)),
		 _ret == sizeof(struct iphdr) + sizeof(struct udphdr) + 1);

	memset(recv_buf, 0, sizeof(recv_buf));
	TEST_RES(
		recvfrom(sk_with_iphdrincl, recv_buf, sizeof(recv_buf), 0,
			 (struct sockaddr *)&recv_addr, &addr_len),
		_ret == sizeof(struct iphdr) + sizeof(struct udphdr) + 1 &&
			recv_buf[sizeof(struct iphdr) + sizeof(struct udphdr)] ==
				'd' &&
			ntohs(recv_udph->dest) == ntohs(udph.dest));

	// TEST CASE 2: Send two messages and receive two messages
	memcpy(send_buf, &iph, sizeof(struct iphdr));
	memcpy(send_buf + sizeof(struct iphdr), &udph, sizeof(struct udphdr));
	send_buf[sizeof(struct iphdr) + sizeof(struct udphdr)] = 'e';
	TEST_RES(sendto(sk_with_iphdrincl, send_buf, htons(iph.tot_len), 0,
			(struct sockaddr *)&dest_addr, sizeof(dest_addr)),
		 _ret == sizeof(struct iphdr) + sizeof(struct udphdr) + 1);
	memcpy(send_buf, &iph, sizeof(struct iphdr));
	memcpy(send_buf + sizeof(struct iphdr), &udph, sizeof(struct udphdr));
	send_buf[sizeof(struct iphdr) + sizeof(struct udphdr)] = 'f';
	TEST_RES(sendto(sk_with_iphdrincl, send_buf, htons(iph.tot_len), 0,
			(struct sockaddr *)&dest_addr, sizeof(dest_addr)),
		 _ret == sizeof(struct iphdr) + sizeof(struct udphdr) + 1);

	memset(recv_buf, 0, sizeof(recv_buf));
	TEST_RES(
		recvfrom(sk_with_iphdrincl, recv_buf, sizeof(recv_buf), 0,
			 (struct sockaddr *)&recv_addr, &addr_len),
		_ret == sizeof(struct iphdr) + sizeof(struct udphdr) + 1 &&
			recv_buf[sizeof(struct iphdr) + sizeof(struct udphdr)] ==
				'e' &&
			ntohs(recv_udph->dest) == ntohs(udph.dest));
	memset(recv_buf, 0, sizeof(recv_buf));
	TEST_RES(
		recvfrom(sk_with_iphdrincl, recv_buf, sizeof(recv_buf), 0,
			 (struct sockaddr *)&recv_addr, &addr_len),
		_ret == sizeof(struct iphdr) + sizeof(struct udphdr) + 1 &&
			recv_buf[sizeof(struct iphdr) + sizeof(struct udphdr)] ==
				'f' &&
			ntohs(recv_udph->dest) == ntohs(udph.dest));
}
END_TEST()

FN_TEST(sendmsg_and_recvmsg_with_iphdrincl)
{
	char send_buf[4096];
	char recv_buf[4096];
	socklen_t addr_len = sizeof(recv_addr);

	struct iphdr iph;
	struct udphdr udph;

	memset(send_buf, 0, sizeof(send_buf));
	memset(recv_buf, 0, sizeof(recv_buf));
	iph.ihl = 5;
	iph.version = 4;
	iph.tos = 0;
	iph.tot_len = htons(sizeof(struct iphdr) + sizeof(struct udphdr) + 1);
	iph.id = htonl(54321);
	iph.frag_off = 0;
	iph.ttl = 255;
	iph.protocol = IPPROTO_UDP;
	iph.check = 0;
	iph.saddr = INADDR_ANY;
	iph.daddr = dest_addr.sin_addr.s_addr;
	iph.check = checksum((unsigned short *)&iph, sizeof(struct iphdr));

	udph.source = htons(54321);
	udph.dest = C_PORT;
	udph.len = htons(sizeof(struct udphdr) + 1);
	udph.check = 0;
	struct udphdr *recv_udph =
		(struct udphdr *)(recv_buf + sizeof(struct iphdr));

	// Avoid interference from previous packets on the network.
	for (int i = 0; i < 9; i++) {
		if (recvfrom(sk_with_iphdrincl, recv_buf, sizeof(recv_buf), 0,
			     (struct sockaddr *)&recv_addr, &addr_len) <= 0)
			break;
	}
	sleep(1);

	// Prepare msg_send and msg_recv
	struct msghdr msg_send = { 0 };
	struct msghdr msg_recv = { 0 };
	struct iovec iov_send = { 0 };
	struct iovec iov_recv = { 0 };

	// TEST CASE 1: Send one message and receive one message
	memcpy(send_buf, &iph, sizeof(struct iphdr));
	memcpy(send_buf + sizeof(struct iphdr), &udph, sizeof(struct udphdr));
	send_buf[sizeof(struct iphdr) + sizeof(struct udphdr)] = 'd';
	iov_send.iov_base = send_buf;
	iov_send.iov_len = htons(iph.tot_len);
	msg_send.msg_name = &dest_addr;
	msg_send.msg_namelen = sizeof(dest_addr);
	msg_send.msg_iov = &iov_send;
	msg_send.msg_iovlen = 1;

	TEST_RES(sendmsg(sk_with_iphdrincl, &msg_send, 0),
		 _ret == sizeof(struct iphdr) + sizeof(struct udphdr) + 1);

	memset(recv_buf, 0, sizeof(recv_buf));
	char name_buf[sizeof(struct sockaddr_in)];
	msg_recv.msg_name = name_buf;
	msg_recv.msg_namelen = sizeof(name_buf);
	iov_recv.iov_base = recv_buf;
	iov_recv.iov_len = sizeof(recv_buf);
	msg_recv.msg_iov = &iov_recv;
	msg_recv.msg_iovlen = 1;

	TEST_RES(
		recvmsg(sk_with_iphdrincl, &msg_recv, 0),
		_ret == sizeof(struct iphdr) + sizeof(struct udphdr) + 1 &&
			recv_buf[sizeof(struct iphdr) + sizeof(struct udphdr)] ==
				'd' &&
			ntohs(recv_udph->dest) == ntohs(udph.dest));

	// Verify msg.msg_name (address and port)
	struct sockaddr_in *recv_name = (struct sockaddr_in *)msg_recv.msg_name;
	TEST_RES(recv_name->sin_addr.s_addr == dest_addr.sin_addr.s_addr, true);
	TEST_RES(recv_name->sin_port == dest_addr.sin_port, true);

	// TEST CASE 2: Send two messages and receive two messages
	memcpy(send_buf, &iph, sizeof(struct iphdr));
	memcpy(send_buf + sizeof(struct iphdr), &udph, sizeof(struct udphdr));
	send_buf[sizeof(struct iphdr) + sizeof(struct udphdr)] = 'e';
	TEST_RES(sendmsg(sk_with_iphdrincl, &msg_send, 0),
		 _ret == sizeof(struct iphdr) + sizeof(struct udphdr) + 1);

	memcpy(send_buf, &iph, sizeof(struct iphdr));
	memcpy(send_buf + sizeof(struct iphdr), &udph, sizeof(struct udphdr));
	send_buf[sizeof(struct iphdr) + sizeof(struct udphdr)] = 'f';
	TEST_RES(sendmsg(sk_with_iphdrincl, &msg_send, 0),
		 _ret == sizeof(struct iphdr) + sizeof(struct udphdr) + 1);

	memset(recv_buf, 0, sizeof(recv_buf));
	TEST_RES(
		recvmsg(sk_with_iphdrincl, &msg_recv, 0),
		_ret == sizeof(struct iphdr) + sizeof(struct udphdr) + 1 &&
			recv_buf[sizeof(struct iphdr) + sizeof(struct udphdr)] ==
				'e' &&
			ntohs(recv_udph->dest) == ntohs(udph.dest));

	// Verify msg.msg_name (address and port) for second message
	recv_name = (struct sockaddr_in *)msg_recv.msg_name;
	TEST_RES(recv_name->sin_addr.s_addr == dest_addr.sin_addr.s_addr, true);
	TEST_RES(recv_name->sin_port == dest_addr.sin_port, true);

	memset(recv_buf, 0, sizeof(recv_buf));
	TEST_RES(
		recvmsg(sk_with_iphdrincl, &msg_recv, 0),
		_ret == sizeof(struct iphdr) + sizeof(struct udphdr) + 1 &&
			recv_buf[sizeof(struct iphdr) + sizeof(struct udphdr)] ==
				'f' &&
			ntohs(recv_udph->dest) == ntohs(udph.dest));

	// Verify msg.msg_name (address and port) for third message
	recv_name = (struct sockaddr_in *)msg_recv.msg_name;
	TEST_RES(recv_name->sin_addr.s_addr == dest_addr.sin_addr.s_addr, true);
	TEST_RES(recv_name->sin_port == dest_addr.sin_port, true);
}
END_TEST()

void set_nonblocking(int sockfd)
{
	int flags = CHECK(fcntl(sockfd, F_GETFL, 0));
	CHECK(fcntl(sockfd, F_SETFL, flags | O_NONBLOCK));
}

FN_SETUP(enter_nonblocking_mode)
{
	set_nonblocking(sk_connected);
	set_nonblocking(sk_bound);
}
END_SETUP()
