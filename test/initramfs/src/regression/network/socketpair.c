// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/socket.h>
#include <unistd.h>
#define MESG1 "Hello from child"
#define MESG2 "Hello from parent"

static void check_fionread(int type)
{
	int sockets[2];
	int pending = -1;
	char buf[8];

	if (socketpair(AF_UNIX, type, 0, sockets) < 0) {
		perror("create socket pair for FIONREAD");
		exit(1);
	}

	if (ioctl(sockets[1], FIONREAD, &pending) < 0) {
		perror("FIONREAD on empty socket");
		exit(1);
	}
	if (pending != 0) {
		fprintf(stderr, "FIONREAD on empty socket got %d\n", pending);
		exit(1);
	}

	if (send(sockets[0], "abc", 3, 0) != 3) {
		perror("send for FIONREAD");
		exit(1);
	}
	if (ioctl(sockets[1], FIONREAD, &pending) < 0) {
		perror("FIONREAD on readable socket");
		exit(1);
	}
	if (pending != 3) {
		fprintf(stderr, "FIONREAD on readable socket got %d\n",
			pending);
		exit(1);
	}

	if (recv(sockets[1], buf, sizeof(buf), 0) < 0) {
		perror("recv for FIONREAD");
		exit(1);
	}
	if (ioctl(sockets[1], FIONREAD, &pending) < 0) {
		perror("FIONREAD after recv");
		exit(1);
	}
	if (pending != 0) {
		fprintf(stderr, "FIONREAD after recv got %d\n", pending);
		exit(1);
	}

	close(sockets[0]);
	close(sockets[1]);
}

int main()
{
	int sockets[2], child;
	char buf[1024];
	check_fionread(SOCK_STREAM);
	check_fionread(SOCK_DGRAM);
	check_fionread(SOCK_SEQPACKET);

	if (socketpair(AF_UNIX, SOCK_STREAM, 0, sockets) < 0) {
		perror("create socket pair");
		exit(1);
	}
	if ((child = fork()) == -1)
		perror("fork");
	else if (child) {
		// parent
		close(sockets[0]);
		if (read(sockets[1], buf, 1024) < 0)
			perror("read from child");
		printf("Receive from child: %s\n", buf);
		if (write(sockets[1], MESG2, sizeof(MESG2)) < 0)
			perror("write to child");
		close(sockets[1]);
	} else {
		// child
		close(sockets[1]);
		if (write(sockets[0], MESG1, sizeof(MESG1)) < 0)
			perror("write to parent");
		if (read(sockets[0], buf, 1024) < 0)
			perror("read from parent");
		printf("Receive from parent: %s\n", buf);
		close(sockets[0]);
	}
	return 0;
}
