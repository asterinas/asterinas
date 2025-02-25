// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <sched.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

int main()
{
	unsigned int cpu, node;
	if (getcpu(&cpu, &node) != 0) {
		perror("getcpu");
		exit(EXIT_FAILURE);
	}
	printf("CPU ID: %d\n", cpu);
	printf("Node ID: %d\n", node);
	return 0;
}
