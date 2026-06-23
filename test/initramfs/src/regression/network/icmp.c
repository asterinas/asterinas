// SPDX-License-Identifier: MPL-2.0

#include <unistd.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <netinet/in.h>
#include <netinet/ip.h>
#include <netinet/ip_icmp.h>
#include <arpa/inet.h>
#include <string.h>
#include <stdint.h>
#include <stdlib.h>

#include "../common/test.h"

#define TEST_ADDR "127.0.0.1"
#define PING_COUNT 3

static int icmp_ident;

FN_SETUP(general)
{
	icmp_ident = (uint16_t)getpid() & 0xffff;
}
END_SETUP()

FN_TEST(icmp_ping_loopback)
{
	/* Create a raw ICMP socket */
	int sk = TEST_SUCC(socket(AF_INET, SOCK_RAW, IPPROTO_ICMP));

	/* Set timeout for receiving */
	struct timeval tv = { .tv_sec = 5, .tv_usec = 0 };
	TEST_SUCC(setsockopt(sk, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv)));

	for (uint16_t seq = 1; seq <= PING_COUNT; seq++) {
		/* Construct ICMP echo request */
		struct icmphdr icmp_hdr;
		memset(&icmp_hdr, 0, sizeof(icmp_hdr));
		icmp_hdr.type = ICMP_ECHO;
		icmp_hdr.code = 0;
		icmp_hdr.un.echo.id = htons(icmp_ident);
		icmp_hdr.un.echo.sequence = htons(seq);

		/* Calculate checksum */
		icmp_hdr.checksum = 0;
		uint32_t sum = 0;
		uint16_t *p = (uint16_t *)&icmp_hdr;
		for (size_t i = 0; i < sizeof(icmp_hdr) / 2; i++) {
			sum += ntohs(p[i]);
		}
		while (sum >> 16) {
			sum = (sum & 0xffff) + (sum >> 16);
		}
		icmp_hdr.checksum = htons((uint16_t)(~sum));

		/* Send echo request to 127.0.0.1 */
		struct sockaddr_in dst;
		dst.sin_family = AF_INET;
		dst.sin_port = 0;
		CHECK(inet_aton(TEST_ADDR, &dst.sin_addr));

		TEST_RES(sendto(sk, &icmp_hdr, sizeof(icmp_hdr), 0,
				(struct sockaddr *)&dst, sizeof(dst)),
			 _ret == sizeof(icmp_hdr));

		/* Receive the reply */
		char recv_buf[512];
		struct sockaddr_in src_addr;
		socklen_t addrlen = sizeof(src_addr);

		int received = TEST_SUCC(
			recvfrom(sk, recv_buf, sizeof(recv_buf), 0,
				 (struct sockaddr *)&src_addr, &addrlen));

		/* Verify the reply */
		struct iphdr *ip = (struct iphdr *)recv_buf;
		struct icmphdr *reply_icmp =
			(struct icmphdr *)(recv_buf + ip->ihl * 4);

		TEST_RES(received, received >= (int)(ip->ihl * 4 +
						     sizeof(struct icmphdr)));
		TEST_RES(reply_icmp->type, reply_icmp->type == ICMP_ECHOREPLY);
		TEST_RES(reply_icmp->code, reply_icmp->code == 0);
		TEST_RES(ntohs(reply_icmp->un.echo.id),
			 ntohs(reply_icmp->un.echo.id) == icmp_ident);
		TEST_RES(ntohs(reply_icmp->un.echo.sequence),
			 ntohs(reply_icmp->un.echo.sequence) == seq);
	}

	TEST_SUCC(close(sk));
}
END_TEST()

FN_TEST(icmp_echo_loopback)
{
	/* Create a raw ICMP socket to send and receive echo on loopback */
	int sk = TEST_SUCC(socket(AF_INET, SOCK_RAW, IPPROTO_ICMP));

	/* Set timeout for receiving */
	struct timeval tv = { .tv_sec = 3, .tv_usec = 0 };
	TEST_SUCC(setsockopt(sk, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv)));

	/* Construct ICMP echo request */
	struct icmphdr icmp_hdr;
	memset(&icmp_hdr, 0, sizeof(icmp_hdr));
	icmp_hdr.type = ICMP_ECHO;
	icmp_hdr.code = 0;
	icmp_hdr.un.echo.id = htons(icmp_ident);
	icmp_hdr.un.echo.sequence = htons(1);

	/* Calculate checksum (simple, 0 is also accepted by most stacks) */
	icmp_hdr.checksum = 0;
	uint32_t sum = 0;
	uint16_t *p = (uint16_t *)&icmp_hdr;
	for (size_t i = 0; i < sizeof(icmp_hdr) / 2; i++) {
		sum += ntohs(p[i]);
	}
	while (sum >> 16) {
		sum = (sum & 0xffff) + (sum >> 16);
	}
	icmp_hdr.checksum = htons((uint16_t)(~sum));

	/* Send echo request to 127.0.0.1 */
	struct sockaddr_in dst;
	dst.sin_family = AF_INET;
	dst.sin_port = 0;
	CHECK(inet_aton("127.0.0.1", &dst.sin_addr));

	TEST_RES(sendto(sk, &icmp_hdr, sizeof(icmp_hdr), 0,
			(struct sockaddr *)&dst, sizeof(dst)),
		 _ret == sizeof(icmp_hdr));

	/* Receive the reply (loopback should deliver ICMP_ECHOREPLY) */
	char recv_buf[512];
	struct sockaddr_in src_addr;
	socklen_t addrlen = sizeof(src_addr);

	int received =
		TEST_SUCC(recvfrom(sk, recv_buf, sizeof(recv_buf), 0,
				   (struct sockaddr *)&src_addr, &addrlen));

	/* The received packet should contain IP header + ICMP header */
	struct iphdr *ip = (struct iphdr *)recv_buf;
	struct icmphdr *reply_icmp = (struct icmphdr *)(recv_buf + ip->ihl * 4);

	TEST_RES(received,
		 received >= (int)(ip->ihl * 4 + sizeof(struct icmphdr)));
	TEST_RES(reply_icmp->type, reply_icmp->type == ICMP_ECHOREPLY);
	TEST_RES(reply_icmp->code, reply_icmp->code == 0);
	TEST_RES(ntohs(reply_icmp->un.echo.id),
		 ntohs(reply_icmp->un.echo.id) == icmp_ident);
	TEST_RES(ntohs(reply_icmp->un.echo.sequence),
		 ntohs(reply_icmp->un.echo.sequence) == 1);

	TEST_SUCC(close(sk));
}
END_TEST()
