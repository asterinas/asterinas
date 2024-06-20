// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/mman.h>
#include <sys/wait.h>

// Make the value not at the page boundary to uncover more bugs
#define OFFSET 2

void do_test(int shared_flag, int init_value, int new_value, int expected_value)
{
	volatile int *shared_memory;

	// Create an anonymous memory region for sharing
	shared_memory = mmap(NULL, sizeof(int), PROT_READ | PROT_WRITE,
			     shared_flag | MAP_ANONYMOUS, -1, 0);
	if (shared_memory == MAP_FAILED) {
		perror("mmap failed");
		exit(1);
	}

	if (init_value != 0) {
		shared_memory[OFFSET] = init_value;
	}

	// Create a child process
	pid_t pid = fork();

	if (pid < 0) {
		perror("fork");
		exit(1);
	} else if (pid == 0) {
		// Child process

		// Modify the value in the shared memory in the child process
		shared_memory[OFFSET] = new_value;

		printf("Child process: Value in shared memory: %d\n",
		       shared_memory[OFFSET]);

		exit(0);
	} else {
		// Parent process

		// Wait for the child process to finish
		int child_status = -1;
		if (wait(&child_status) < 0) {
			perror("wait");
			exit(1);
		}

		if (child_status != 0) {
			printf("child process terminates abnormally\n");
			exit(1);
		}

		printf("Parent process: Value in shared memory: %d\n",
		       shared_memory[OFFSET]);

		if (shared_memory[OFFSET] != expected_value) {
			perror("shared memory contains invalid value");
			exit(1);
		}

		// Unmap the memory
		if (munmap((void *)shared_memory, sizeof(int)) == -1) {
			perror("munmap failed");
			exit(1);
		}
	}
}

int main(void)
{
	printf("Fork non-accessed shared region\n");
	do_test(MAP_SHARED, 0, 123, 123);

	printf("Fork accessed shared region\n");
	do_test(MAP_SHARED, 1, 123, 123);

	printf("Fork non-accessed private region\n");
	do_test(MAP_PRIVATE, 0, 123, 0);

	printf("Fork accessed private region\n");
	do_test(MAP_PRIVATE, 1, 123, 1);

	return 0;
}
