// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <sys/prctl.h>
#include <signal.h>
#include <stdlib.h>
#include <stdio.h>
#include <unistd.h>

void signal_handler(int signum)
{
	if (signum == SIGTERM) {
		printf("child process reveives SIGTERM\n");
		exit(EXIT_SUCCESS);
	}
}

int main()
{
	pid_t pid = fork();

	if (pid == -1) {
		perror("fork");
		return EXIT_FAILURE;
	}

	if (pid > 0) {
		printf("Parent PID: %d\n", getpid());
		// Ensure parent won't exit before child process runs
		sleep(1);
	} else {
		printf("CHild PID: %d\n", getpid());

		prctl(PR_SET_PDEATHSIG, SIGTERM);

		struct sigaction sa;
		sa.sa_handler = signal_handler;
		sigemptyset(&sa.sa_mask);
		sa.sa_flags = 0;
		sigaction(SIGTERM, &sa, NULL);

		// Child waits for signal from parent death
		while (1) {
			sleep(1);
		}
	}

	return EXIT_SUCCESS;
}