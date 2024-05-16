// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/epoll.h>
#include <sys/wait.h>
#include <string.h>

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
		return EXIT_FAILURE;
	}

	// Create epoll instance
	if ((epfd = epoll_create1(0)) == -1) {
		perror("epoll_create1 error");
		close(pipefd[0]);
		close(pipefd[1]);
		return EXIT_FAILURE;
	}

	// Fork to create child process
	cpid = fork();
	if (cpid == -1) {
		perror("fork error");
		close(pipefd[0]);
		close(pipefd[1]);
		close(epfd);
		return EXIT_FAILURE;
	}

	if (cpid == 0) { // Child process
		close(pipefd[0]); // Child closes read end of the pipe
		const char *message = "Hello, world!\n";
		write(pipefd[1], message,
		      strlen(message)); // Write a string to the pipe
		close(pipefd[1]); // Close write end of the pipe
		_exit(EXIT_SUCCESS);
	} else { // Parent process
		close(pipefd[1]); // Parent closes write end of the pipe

		// Set up epoll to listen for events
		ev.events = EPOLLIN; // Listen for input events
		ev.data.fd = pipefd[0];
		if (epoll_ctl(epfd, EPOLL_CTL_ADD, pipefd[0], &ev) == -1) {
			perror("epoll_ctl error");
			close(pipefd[0]);
			close(epfd);
			return EXIT_FAILURE;
		}

		// Wait for events to occur
		printf("Waiting for event on pipe...\n");
		nfds = epoll_wait(epfd, events, 1, -1);
		if (nfds == -1) {
			perror("epoll_wait error");
			close(pipefd[0]);
			close(epfd);
			return EXIT_FAILURE;
		}

		// If we get here, epoll_wait was successful
		printf("epoll_wait returned successfully.\n");
		if (events[0].data.fd == pipefd[0]) {
			// Read data
			ssize_t count = read(pipefd[0], buf, sizeof(buf) - 1);
			if (count > 0) {
				buf[count] =
					'\0'; // Ensure string is null-terminated
				printf("Received data: %s",
				       buf); // Output the entire string
			} else {
				perror("read error");
			}
		}

		close(pipefd[0]); // Close read end of the pipe
		close(epfd); // Close the epoll file descriptor
	}

	// Wait for the child process to complete
	wait(NULL);

	return EXIT_SUCCESS;
}
