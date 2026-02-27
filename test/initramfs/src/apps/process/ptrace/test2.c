#define _GNU_SOURCE
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

int main(void)
{
	printf("[test2] pid=%d, entered after execve\n", getpid());
	fflush(stdout);

	for (int i = 1; i <= 3; i++) {
		sleep(1);
		printf("[test2] alive\n");
		fflush(stdout);
	}

	raise(SIGCHLD);

	for (int i = 1; i <= 3; i++) {
		sleep(1);
		printf("[test2] alive after stop\n");
		fflush(stdout);
	}

	exit(3);
}
