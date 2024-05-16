// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/epoll.h>
#include <signal.h>
#include <sys/wait.h>
#include <string.h>

// Signal handler for SIGUSR1
static void handle_sigusr1(int sig)
{
	write(STDOUT_FILENO, "SIGUSR1 handled\n", 16);
}

int main(void)
{
	int pipefd[2]; // Array to store pipe file descriptors
	pid_t cpid; // Child process ID
	char buf[1024]; // Read buffer
	struct epoll_event ev, events[1];
	int epfd, nfds;

	// Create a pipe
	if (pipe(pipefd) == -1) {
		perror("pipe error");
		exit(EXIT_FAILURE);
	}

	// Create epoll instance
	if ((epfd = epoll_create1(0)) == -1) {
		perror("epoll_create1 error");
		exit(EXIT_FAILURE);
	}

	// Fork to create child process
	cpid = fork();
	if (cpid == -1) {
		perror("fork error");
		exit(EXIT_FAILURE);
	}

	if (cpid == 0) { // Child process
		close(pipefd[0]); // Child closes read end of the pipe

		sleep(3); // Sleep for several seconds to provide a time window to send SIGUSR1

		const char *message = "Message from child process\n";
		write(pipefd[1], message,
		      strlen(message)); // Write a string to the pipe
		close(pipefd[1]); // Close write end of the pipe
		_exit(EXIT_SUCCESS);
	} else {
		// Parent process
		struct sigaction sa;
		sigset_t sigset;

		// Setup signal handler for SIGUSR1
		sa.sa_handler = handle_sigusr1;
		sa.sa_flags = 0;
		sigemptyset(&sa.sa_mask);
		if (sigaction(SIGUSR1, &sa, NULL) == -1) {
			perror("sigaction error");
			exit(EXIT_FAILURE);
		}

		// Prepare the signal set to block SIGUSR1
		sigemptyset(&sigset);
		sigaddset(&sigset, SIGUSR1);

		close(pipefd[1]); // Parent closes write end of the pipe

		// Set up epoll to listen for events
		ev.events = EPOLLIN; // Listen for input events
		ev.data.fd = pipefd[0];
		if (epoll_ctl(epfd, EPOLL_CTL_ADD, pipefd[0], &ev) == -1) {
			perror("epoll_ctl error");
			exit(EXIT_FAILURE);
		}

		// Wait for events to occur, blocking SIGUSR1
		printf("Waiting for event on pipe, SIGUSR1 is blocked...\n");
		nfds = epoll_pwait(epfd, events, 1, -1, &sigset);
		if (nfds == -1) {
			perror("epoll_pwait error");
			exit(EXIT_FAILURE);
		}

		// If we get here, epoll_pwait was successful
		printf("epoll_pwait returned successfully.\n");
		if (events[0].data.fd == pipefd[0]) {
			// Read data
			ssize_t count = read(pipefd[0], buf, sizeof(buf) - 1);
			if (count > 0) {
				buf[count] =
					'\0'; // Ensure string is null-terminated
				printf("Received data: %s",
				       buf); // Output the entire string
			}
		}

		close(pipefd[0]); // Close read end of the pipe
		close(epfd); // Close the epoll file descriptor
	}

	// Wait for the child process to complete
	wait(NULL);

	return EXIT_SUCCESS;
}
