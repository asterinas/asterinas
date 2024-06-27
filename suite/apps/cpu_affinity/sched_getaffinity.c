// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <stdio.h>
#include <unistd.h>
#include <syscall.h>
#include <errno.h>
#include <sched.h> // Include sched.h for CPU_SETSIZE

int main()
{
	// Create a mask for CPU_SETSIZE number of CPUs
	unsigned long mask[CPU_SETSIZE / sizeof(unsigned long)];
	int mask_size = sizeof(mask);

	// Call the raw syscall to retrieve the CPU affinity mask of the current process
	long res = syscall(__NR_sched_getaffinity, 0, mask_size, &mask);

	if (res < 0) {
		perror("Error calling sched_getaffinity");
		return errno;
	}

	// Print the CPUs that are part of the current process's affinity mask
	printf("Process can run on: ");
	for (int i = 0; i < CPU_SETSIZE; ++i) {
		if (mask[i / (8 * sizeof(long))] &
		    (1UL << (i % (8 * sizeof(long))))) {
			printf("%d ", i);
		}
	}
	printf("\n");

	return 0;
}
