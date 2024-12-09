#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <time.h>
#include <string.h>

#define PAGE_SIZE 4096 // Typical page size in bytes
#define MAX_PAGES (1<<23) // 32G
// #define MAX_PAGES (1<<18) // 1G
#define TEST_ITERATIONS 20

long rdtsc(void)
{
	unsigned int hi, lo;
	__asm__ __volatile__("rdtsc" : "=a"(lo), "=d"(hi));
	return ((long)lo) | (((long)hi) << 32);
}

long get_time_in_nanos(long start_tsc, long end_tsc)
{
	// Our setup is a 1.9 GHz CPU
	return (end_tsc - start_tsc) * 10 / 19;
}

long run_test(int num_pages)
{
	size_t region_size = (size_t)num_pages * (size_t)PAGE_SIZE;

	char *region = mmap(NULL, region_size, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
	if (region == MAP_FAILED) {
		perror("mmap failed");
		exit(EXIT_FAILURE);
	}

	// Trigger page fault on every pages
	for (size_t i = 0; i < num_pages; i++) {
		region[i * PAGE_SIZE] = 1;
	}

	long tsc_start, tsc_end;
	tsc_start = rdtsc();

	// Fork
	int pid = fork();
	if (pid == -1) {
		perror("fork failed");
		exit(EXIT_FAILURE);
	} else if (pid == 0) {
		// Child
		exit(EXIT_SUCCESS);
	} else {
		// Parent
		waitpid(pid, NULL, 0);
		tsc_end = rdtsc();
	}

	munmap(region, region_size);

	return get_time_in_nanos(tsc_start, tsc_end);
}

long lat[TEST_ITERATIONS];

int main(int argc, char *argv[])
{
	printf("Pages, p5 lat (ns), Avg lat (ns), p95 lat (ns), Pos err lat (ns2), Neg err lat (ns2)\n");

	for (int num_pages = 1; num_pages <= MAX_PAGES; num_pages <<= 1) {
		for (int i = 0; i < TEST_ITERATIONS; i++) {
			lat[i] = run_test(num_pages);
		}

		// Calculate the p5, average, p95, and variance of the latencies
		long avg = 0;
		for (int i = 0; i < TEST_ITERATIONS; i++) {
			avg += lat[i];
		}
		avg /= TEST_ITERATIONS;
		long posvar2 = 0;
		long numpos = 0;
		long negvar2 = 0;
		long numneg = 0;
		for (int i = 0; i < TEST_ITERATIONS; i++) {
			long diff = lat[i] - avg;
			if (diff > 0) {
				posvar2 += diff * diff;
				numpos++;
			} else {
				negvar2 += diff * diff;
				numneg++;
			}
		}
		posvar2 /= numpos;
		negvar2 /= numneg;

		// Calculate the p5 and p95 latencies
		// bubble sort
		for (int i = 0; i < TEST_ITERATIONS; i++) {
			for (int j = i + 1; j < TEST_ITERATIONS; j++) {
				if (lat[i] > lat[j]) {
					long temp = lat[i];
					lat[i] = lat[j];
					lat[j] = temp;
				}
			}
		}
		long p5 = lat[TEST_ITERATIONS / 20];
		long p95 = lat[TEST_ITERATIONS * 19 / 20];

		printf("%d, %ld, %ld, %ld, %ld, %ld\n", num_pages, p5, avg, p95,
			posvar2, negvar2);
	}

	return 0;
}
