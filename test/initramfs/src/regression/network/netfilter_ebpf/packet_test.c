// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE

#include <errno.h>
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

#include <arpa/inet.h>
#include <netinet/in.h>

#define TEST_PORT 9753

static uint16_t test_port = TEST_PORT;

static int sender = -1;
static int receiver = -1;
static struct sockaddr_in receiver_addr;

static void setup_sockets(void)
{
	sender = socket(AF_INET, SOCK_DGRAM, 0);
	if (sender < 0) {
		perror("socket(sender)");
		exit(1);
	}

	receiver = socket(AF_INET, SOCK_DGRAM | SOCK_NONBLOCK, 0);
	if (receiver < 0) {
		perror("socket(receiver)");
		exit(1);
	}

	memset(&receiver_addr, 0, sizeof(receiver_addr));
	receiver_addr.sin_family = AF_INET;
	receiver_addr.sin_port = htons(test_port);
	if (inet_aton("127.0.0.1", &receiver_addr.sin_addr) == 0) {
		fprintf(stderr, "inet_aton failed\n");
		exit(1);
	}

	if (bind(receiver, (struct sockaddr *)&receiver_addr,
			 sizeof(receiver_addr)) < 0) {
		perror("bind");
		exit(1);
	}
}

static int send_msg(const char *msg)
{
	return sendto(sender, msg, strlen(msg), 0,
		      (struct sockaddr *)&receiver_addr, sizeof(receiver_addr));
}

static int recv_msg_timeout(char *buf, int len, int timeout_ms)
{
	struct pollfd pfd = { .fd = receiver, .events = POLLIN };
	int ready = poll(&pfd, 1, timeout_ms);
	if (ready == 0) {
		errno = EAGAIN;
		return -1;
	}
	if (ready < 0) {
		return -1;
	}
	return recvfrom(receiver, buf, len, 0, NULL, NULL);
}

static void usage(const char *prog)
{
	fprintf(stderr,
		"usage: %s [--expect pass|drop] [-p|--port port] [payload]\n",
		prog);
}

int main(int argc, char **argv)
{
	const char *expect = "pass";
	const char *payload = "demo packet";

	for (int i = 1; i < argc; ++i) {
		if (strcmp(argv[i], "--expect") == 0 && i + 1 < argc) {
			expect = argv[++i];
			continue;
		}
		if ((strcmp(argv[i], "-p") == 0 || strcmp(argv[i], "--port") == 0) &&
		    i + 1 < argc) {
			char *end = NULL;
			long port = strtol(argv[++i], &end, 10);
			if (end == argv[i] || *end != '\0' || port <= 0 || port > UINT16_MAX) {
				fprintf(stderr, "invalid port: %s\n", argv[i]);
				return 2;
			}
			test_port = (uint16_t)port;
			continue;
		}
		if (argv[i][0] == '-') {
			usage(argv[0]);
			return 2;
		}
		payload = argv[i];
	}

	setup_sockets();

	char buf[128];
	while (recv_msg_timeout(buf, sizeof(buf), 0) > 0) {
	}

	if (send_msg(payload) < 0) {
		perror("sendto");
		return 1;
	}

	int received = recv_msg_timeout(buf, sizeof(buf), 500);
	int ok = (received >= 0);

	if (strcmp(expect, "pass") == 0) {
		if (!ok) {
			fprintf(stderr, "expected packet to pass, but it timed out\n");
			return 1;
		}
		printf("packet passed: %.*s\n", received, buf);
	} else if (strcmp(expect, "drop") == 0) {
		if (ok) {
			fprintf(stderr, "expected packet to drop, but it arrived\n");
			return 1;
		}
		printf("packet dropped as expected\n");
	} else {
		usage(argv[0]);
		return 2;
	}

	close(sender);
	close(receiver);
	return 0;
}
