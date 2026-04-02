// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <sched.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <pthread.h>

int main()
{
	pid_t pid = getpid();
	cpu_set_t mask;

	// Get current affinity mask
	if (sched_getaffinity(pid, sizeof(cpu_set_t), &mask) == -1) {
		perror("sched_getaffinity");
		exit(EXIT_FAILURE);
	}

	printf("Current CPU affinity:");
	int cur_cpu_count = 0;
	for (int i = 0; i < CPU_SETSIZE; i++) {
		if (CPU_ISSET(i, &mask)) {
			printf(" %d", i);
			cur_cpu_count++;
		}
	}
	printf("\n");
	if (cur_cpu_count == 0) {
		printf("Error: No CPU affinity set\n");
		exit(EXIT_FAILURE);
	}

	// Set the process to run on CPU 0 only
	CPU_ZERO(&mask);
	CPU_SET(0, &mask);
	if (sched_setaffinity(pid, sizeof(cpu_set_t), &mask) == -1) {
		perror("sched_setaffinity");
		exit(EXIT_FAILURE);
	}
	printf("Set CPU affinity to CPU 0\n");

	// Verify the new CPU affinity
	if (sched_getaffinity(pid, sizeof(cpu_set_t), &mask) == -1) {
		perror("sched_getaffinity");
		exit(EXIT_FAILURE);
	}

	printf("New CPU affinity:");
	cur_cpu_count = 0;
	for (int i = 0; i < CPU_SETSIZE; i++) {
		if (CPU_ISSET(i, &mask)) {
			printf(" %d", i);
			cur_cpu_count++;
			if (i != 0) {
				printf("Error: CPU affinity not set to CPU 0\n");
				exit(EXIT_FAILURE);
			}
		}
	}
	printf("\n");
	if (cur_cpu_count != 1) {
		printf("Error: CPU affinity not set to CPU 0\n");
		exit(EXIT_FAILURE);
	}

	return 0;
}
