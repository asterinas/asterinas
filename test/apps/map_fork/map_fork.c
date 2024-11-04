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
#define MAX_PAGES 262144
#define WARMUP_ITERATIONS 10
#define TEST_ITERATIONS 50

double get_time_in_seconds(struct timespec *start, struct timespec *end)
{
	return (end->tv_sec - start->tv_sec) +
	       (end->tv_nsec - start->tv_nsec) / 1e9;
}

double run_test(int num_pages)
{
	// fallocate -l 1G largefile
	int fd = open("largefile", O_RDWR | O_CREAT, S_IRUSR | S_IWUSR);
	if (fd == -1) {
		perror("open failed");
		exit(EXIT_FAILURE);
	}

	int file_size = num_pages * PAGE_SIZE;

	if (ftruncate(fd, file_size) == -1) {
		perror("ftruncate() error");
	}

	char *region = mmap(NULL, file_size, PROT_READ, MAP_SHARED, fd, 0);
	if (region == MAP_FAILED) {
		perror("mmap failed");
		exit(EXIT_FAILURE);
	}

	close(fd);

	// Trigger page fault on every pages
	for (size_t i = 0; i < num_pages; i++) {
		volatile char c = region[i * PAGE_SIZE];
		c += 1;
	}

	struct timespec time_start, time_end;
	clock_gettime(CLOCK_MONOTONIC, &time_start);

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
		clock_gettime(CLOCK_MONOTONIC, &time_end); // only fork
		waitpid(pid, NULL, 0);
		// clock_gettime(CLOCK_MONOTONIC, &time_end); // fork + exec
	}

	munmap(region, file_size);

	return get_time_in_seconds(&time_start, &time_end);
}

int main()
{
	printf("Pages, Average Time (s)\n");

	for (int num_pages = 1; num_pages <= MAX_PAGES; num_pages <<= 1) {
		for (int i = 0; i < WARMUP_ITERATIONS; i++) {
			run_test(num_pages);
		}

		double total_time = 0.0;

		for (int i = 0; i < TEST_ITERATIONS; i++) {
			total_time += run_test(num_pages);
		}

		double avg_time = total_time / TEST_ITERATIONS;
		printf("%d, %.6f\n", num_pages, avg_time);
	}

	return 0;
}
