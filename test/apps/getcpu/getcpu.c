// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/syscall.h>

int main()
{
	unsigned int cpu, node;

	// Directly test the getcpu syscall because glibc's getcpu() may not
	// use the getcpu syscall to retrieve CPU info
	long ret = syscall(SYS_getcpu, &cpu, &node, NULL);
	if (ret != 0) {
		perror("syscall getcpu");
		exit(EXIT_FAILURE);
	}

	printf("getcpu syscall: cpu = %u, node = %u\n", cpu, node);

	return 0;
}