// SPDX-License-Identifier: MPL-2.0

#define _GNU_SOURCE
#include <stdio.h>
#include <sys/syscall.h>
#include <time.h>
#include <unistd.h>

#define NUM_OF_CALLS 1000000

int main()
{
	struct timespec start, end;
	long seconds, nanoseconds, total_nanoseconds, avg_latency;
	pid_t pid;

	clock_gettime(CLOCK_MONOTONIC, &start);

	for (int i = 0; i < NUM_OF_CALLS; i++) {
		pid = syscall(SYS_getpid);
	}

	clock_gettime(CLOCK_MONOTONIC, &end);

	seconds = end.tv_sec - start.tv_sec;
	nanoseconds = end.tv_nsec - start.tv_nsec;

	total_nanoseconds = seconds * 1e9 + nanoseconds;
	avg_latency = total_nanoseconds / NUM_OF_CALLS;

	printf("Process %d executed the getpid() syscall %d times.\n", pid,
	       NUM_OF_CALLS);
	printf("Syscall average latency: %ld nanoseconds.\n", avg_latency);

	return 0;
}
