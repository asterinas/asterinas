// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/mman.h>
#include <sys/wait.h>

int main()
{
	int *shared_memory;
	int value = 123;

	// Create an anonymous memory region for sharing
	shared_memory = mmap(NULL, sizeof(int), PROT_READ | PROT_WRITE,
			     MAP_SHARED | MAP_ANONYMOUS, -1, 0);
	if (shared_memory == MAP_FAILED) {
		perror("mmap failed");
		exit(1);
	}

	// Create a child process
	pid_t pid = fork();

	if (pid < 0) {
		perror("fork");
		exit(1);
	} else if (pid == 0) {
		// Child process

		// Modify the value in the shared memory in the child process
		*shared_memory = value;

		printf("Child process: Value in shared memory: %d\n",
		       *shared_memory);

	} else {
		// Parent process

		// Wait for the child process to finish
		wait(NULL);

		printf("Parent process: Value in shared memory: %d\n",
		       *shared_memory);

		if (*shared_memory != value) {
			perror("shared memory contains invalid value");
			exit(1);
		}

		// Unmap the memory
		if (munmap(shared_memory, sizeof(int)) == -1) {
			perror("munmap failed");
			exit(1);
		}
	}

	return 0;
}